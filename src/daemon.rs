use std::time::Duration;
use zbus::Connection;
use zbus::fdo::DBusProxy;

use crate::audio;
use crate::config::Config;
use crate::extension_proxy::ExtensionProxy;
use crate::mpris;
use crate::recorder;
use crate::transcriber_factory::create_transcriber;
use crate::vad::VadProcessor;
use crate::accessibility::AccessibilityManager;

#[zbus::proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
trait Login1Manager {
    #[zbus(signal)]
    async fn prepare_for_sleep(&self, active: bool) -> zbus::Result<()>;
}

/// Characters that, when immediately before the cursor, mean we should NOT prepend a space.
fn no_space_before_chars() -> &'static [char] {
    &[
        ' ', '\n', '\t',       // already whitespace
        '(', '[', '{', '<',    // opening brackets
        '"', '\'', '`',        // quotes
        '-', '@', '#', '/', '\\', // mid-word / start-of-token
        '$', '£', '€',         // currency
    ]
}

struct RecordingState {
    samples: Vec<f32>,
    recorder_output: recorder::RecorderOutput,
    original_volume: Option<f64>,
    paused_players: Vec<mpris::PlayerState>,
    config: Config,
    vad_processor: VadProcessor,
    is_speaking: bool,
    cursor_context: Option<String>,
}

pub async fn run_daemon() {
    let conn = Connection::session().await.expect("Failed to connect to session bus");

    // Own a name so the extension can track our presence
    conn.request_name("com.timcharper.dictation.Daemon")
        .await
        .expect("Failed to request daemon name. Is another instance running?");

    let audio_mgr = audio::AudioManager::new();
    let accessibility_mgr = AccessibilityManager::new().await.ok();
    let mpris_client = mpris::MprisClient::new(conn.clone());
    let mut history_mgr = crate::history::HistoryManager::load();

    // Subscribe to system sleep/wake events once — the D-Bus connection survives sleep
    let login_proxy = Login1ManagerProxy::new(&conn).await;
    let mut sleep_stream = match &login_proxy {
        Ok(lp) => match lp.receive_prepare_for_sleep().await {
            Ok(stream) => {
                println!("Subscribed to system sleep/wake events.");
                Some(stream)
            }
            Err(e) => {
                eprintln!("Warning: could not subscribe to sleep events: {e}");
                None
            }
        },
        Err(e) => {
            eprintln!("Warning: could not connect to login1 (sleep detection disabled): {e}");
            None
        }
    };

    // Watch for the extension's bus name disappearing — this fires reliably when
    // GNOME Shell restarts (signal streams just go silent without returning None).
    let dbus_proxy = DBusProxy::new(&conn).await.expect("Failed to connect to org.freedesktop.DBus");
    let mut name_owner_stream = dbus_proxy.receive_name_owner_changed().await
        .expect("Failed to subscribe to NameOwnerChanged");

    loop {
        println!("Connecting to extension...");

        let proxy = match ExtensionProxy::new(&conn).await {
            Ok(p) => p,
            Err(e) => {
                eprintln!("Extension not available: {e}. Retrying in 2s...");
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
        };

        let mut config = Config::load();
        println!("Connected to extension. Shortcut: {}. Initializing...", config.shortcut);

        let get_menu_items = |hm: &crate::history::HistoryManager| {
            let mut items = vec![("settings".to_string(), "Settings".to_string())];
            items.extend(hm.menu_items());
            items
        };

        if let Err(e) = proxy.update("audio-input-microphone-symbolic", get_menu_items(&history_mgr), "idle", "").await {
            eprintln!("Failed initial extension update: {e}. Retrying in 2s...");
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        }

        if let Err(e) = proxy.register_shortcut(&config.shortcut).await {
            eprintln!("Failed to register shortcut: {e}. Retrying in 2s...");
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        }

        let mut menu_stream = match proxy.receive_menu_item_selected().await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Failed to subscribe to menu signals: {e}. Retrying in 2s...");
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
        };

        let mut shortcut_stream = match proxy.receive_shortcut_pressed().await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Failed to subscribe to shortcut signals: {e}. Retrying in 2s...");
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
        };

        println!("Daemon ready. Listening for shortcuts...");

        let mut recording_state: Option<RecordingState> = None;
        let mut wm_at_last_focus: Option<String> = None;
        let mut focus_rx = accessibility_mgr.as_ref().map(|m| m.focus_receiver.clone());

        let config_path = Config::path();
        let (config_tx, mut config_rx) = tokio::sync::mpsc::channel::<()>(1);
        let _config_watcher = {
            use notify::{Watcher, RecursiveMode, EventKind, event::ModifyKind};
            let tx = config_tx.clone();
            let mut w = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                if let Ok(event) = res {
                    if matches!(event.kind, EventKind::Modify(ModifyKind::Data(_))) {
                        let _ = tx.try_send(());
                    }
                }
            }).expect("Failed to create config file watcher");
            w.watch(&config_path, RecursiveMode::NonRecursive)
                .expect("Failed to watch config file");
            w
        };

        loop {
            tokio::select! {
                _ = async {
                    if let Some(rx) = &mut focus_rx {
                        rx.changed().await.ok();
                    } else {
                        std::future::pending::<()>().await;
                    }
                } => {
                    wm_at_last_focus = proxy.get_focused_window_class().await.ok();
                }
                sleep_signal = async {
                    if let Some(s) = &mut sleep_stream {
                        tokio_stream::StreamExt::next(s).await
                    } else {
                        std::future::pending::<Option<_>>().await
                    }
                } => {
                    if let Some(sig) = sleep_signal {
                        if let Ok(args) = sig.args() {
                            if args.active {
                                println!("System preparing to sleep.");
                                if recording_state.is_some() {
                                    eprintln!("Warning: recording in progress when system went to sleep; discarding.");
                                    recording_state = None;
                                }
                            } else {
                                println!("System waking from sleep. Reconnecting to extension...");
                                break;
                            }
                        }
                    } else {
                        eprintln!("Sleep event stream ended unexpectedly.");
                        // Don't break — keep running without sleep detection
                        sleep_stream = None;
                    }
                }
                name_owner_signal = tokio_stream::StreamExt::next(&mut name_owner_stream) => {
                    if let Some(sig) = name_owner_signal {
                        if let Ok(args) = sig.args() {
                            if args.name == "com.timcharper.dictation.Extension" && args.new_owner.as_deref().unwrap_or("").is_empty() {
                                println!("Extension bus name disappeared (GNOME Shell restart?). Reconnecting...");
                                break;
                            }
                        }
                    }
                }
                menu_signal = tokio_stream::StreamExt::next(&mut menu_stream) => {
                    match menu_signal {
                        Some(signal) => {
                            let args = signal.args().expect("Failed to parse signal args");
                            if args.id == "settings" {
                                println!("Opening settings dialog...");
                                let current_exe = std::env::current_exe().expect("Failed to get current exe");
                                let _ = std::process::Command::new(current_exe).spawn();
                            } else if args.id.starts_with("history_") {
                                if let Ok(index) = args.id["history_".len()..].parse::<usize>() {
                                    if let Some(entry) = history_mgr.entries.get(index) {
                                        println!("Copying history item {} to clipboard", index);
                                        let _ = proxy.set_clipboard(&entry.text).await;
                                    }
                                }
                            }
                        }
                        None => {
                            eprintln!("Menu signal stream ended; extension disconnected. Reconnecting...");
                            break;
                        }
                    }
                }
                shortcut_signal = tokio_stream::StreamExt::next(&mut shortcut_stream) => {
                    match shortcut_signal {
                        Some(_) => {
                            if let Some(state) = recording_state.take() {
                                println!("Shortcut pressed! Stopping recording and transcribing...");
                                let stop_time = std::time::Instant::now();

                                // Signal transcribing state
                                let _ = proxy.update("emblem-synchronizing-symbolic", get_menu_items(&history_mgr), "transcribing", &state.config.transcribing_color).await;

                                // Restore volume and play end sound immediately
                                audio_mgr.restore_and_play_end(&proxy, &state.config.sound, state.original_volume).await;

                                let format = crate::traits::AudioFormat {
                                    sample_rate: state.recorder_output.config.sample_rate.0,
                                    channels: state.recorder_output.config.channels as u16,
                                };
                                let raw_bytes = bytes::Bytes::from(bytemuck::cast_slice::<f32, u8>(&state.samples).to_vec());

                                let transcriber = create_transcriber(&state.config.backend, state.cursor_context.clone());

                                println!("Transcribing...");
                                let bytes_only_stream = futures_util::stream::once(async move { raw_bytes });

                                match transcriber.stream_transcription(format, Box::pin(bytes_only_stream)).await {
                                    Ok(mut transcription_stream) => {
                                        let mut full_text = String::new();
                                        while let Some(res) = futures_util::StreamExt::next(&mut transcription_stream).await {
                                            match res {
                                                Ok(resp) => {
                                                    if resp.is_final {
                                                        full_text = resp.text;
                                                    } else if !resp.text.is_empty() {
                                                        full_text = resp.text;
                                                    }
                                                },
                                                Err(e) => eprintln!("Transcription error: {:?}", e),
                                            }
                                        }

                                        if !full_text.is_empty() {
                                            let elapsed = stop_time.elapsed();
                                            let delay = std::time::Duration::from_millis(state.config.typing_delay_ms);
                                            if elapsed < delay {
                                                let remaining = delay - elapsed;
                                                println!("Transcribed: '{}'. Delaying {}ms more before typing...", full_text, remaining.as_millis());
                                                tokio::time::sleep(remaining).await;
                                            } else {
                                                println!("Transcribed: '{}'. Typing immediately.", full_text);
                                            }
                                            let text_to_type = match state.cursor_context.as_deref().and_then(|s| s.chars().last()) {
                                                Some(c) if !no_space_before_chars().contains(&c) => format!(" {}", full_text),
                                                _ => full_text.clone(),
                                            };
                                            let _ = proxy.type_string(&text_to_type).await;
                                            history_mgr.add_entry(full_text);
                                        } else {
                                            println!("No text transcribed.");
                                        }
                                    },
                                    Err(e) => eprintln!("Failed to start transcription: {:?}", e),
                                }

                                // Resume MPRIS
                                for p_state in state.paused_players {
                                    if let Ok(player_proxy) = mpris_client.get_proxy(&p_state.service).await {
                                        if let Ok(status) = player_proxy.playback_status().await {
                                            if status == "Paused" {
                                                if let Ok(metadata) = player_proxy.metadata().await {
                                                    let current_track_id = mpris::extract_track_id(&metadata);
                                                    if current_track_id == p_state.track_id {
                                                        println!("Resuming player: {}", p_state.service);
                                                        let _ = player_proxy.play().await;
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                println!("Dictation cycle complete.");
                                let _ = proxy.update("audio-input-microphone-symbolic", get_menu_items(&history_mgr), "idle", "").await;
                            } else {
                                println!("Shortcut pressed! Starting recording...");
                                let config = Config::load();

                                let _ = proxy.update("media-record-symbolic", get_menu_items(&history_mgr), "recording", &config.recording_color).await;

                                // 1. MPRIS Pause
                                let mut paused_players = Vec::new();
                                if let Ok(players) = mpris_client.find_players().await {
                                    for service in players {
                                        if let Ok(player_proxy) = mpris_client.get_proxy(&service).await {
                                            if let Ok(status) = player_proxy.playback_status().await {
                                                if status == "Playing" {
                                                    if let Ok(metadata) = player_proxy.metadata().await {
                                                        let track_id = mpris::extract_track_id(&metadata);
                                                        println!("Pausing player: {} (track: {})", service, track_id);
                                                        let _ = player_proxy.pause().await;
                                                        paused_players.push(mpris::PlayerState {
                                                            service: service.clone(),
                                                            track_id,
                                                        });
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }

                                // 2. Duck volume and play start sound
                                let original_volume = audio_mgr.duck_and_play_start(&proxy, &config.sound).await;

                                // 3. Start Recorder
                                let recorder = recorder::AudioRecorder::new();
                                let output = recorder.start_recording();
                                
                                let vad_processor = VadProcessor::new(
                                    output.config.sample_rate.0,
                                    output.config.channels as u16,
                                );

                                let mut cursor_context = None;
                                if let Some(mgr) = &accessibility_mgr {
                                    let current_wm = proxy.get_focused_window_class().await.ok().unwrap_or_default();

                                    let is_stale = match &wm_at_last_focus {
                                        Some(at_focus_wm) if at_focus_wm != &current_wm => {
                                            eprintln!("[AT-SPI] Skipping stale context: focused window is '{current_wm}' but last AT-SPI focus was for '{at_focus_wm}'");
                                            true
                                        }
                                        _ => false,
                                    };

                                    let is_blacklisted = !is_stale && !current_wm.is_empty() && config.accessibility_blacklist.iter().any(|pattern| {
                                        regex::Regex::new(pattern)
                                            .map(|re| re.is_match(&current_wm))
                                            .unwrap_or(false)
                                    });
                                    if is_blacklisted {
                                        eprintln!("[AT-SPI] Skipping context for blacklisted WM class: {current_wm}");
                                    }

                                    if !is_stale && !is_blacklisted {
                                        match mgr.get_cursor_info().await {
                                            Ok(Some(info)) => cursor_context = Some(info.text_before),
                                            Ok(None) => {}
                                            Err(e) => eprintln!("[AT-SPI] get_cursor_info error: {e}"),
                                        }
                                    }
                                }

                                recording_state = Some(RecordingState {
                                    samples: Vec::new(),
                                    recorder_output: output,
                                    original_volume,
                                    paused_players,
                                    config,
                                    vad_processor,
                                    is_speaking: false,
                                    cursor_context,
                                });
                            }
                        }
                        None => {
                            eprintln!("Shortcut signal stream ended; extension disconnected. Reconnecting...");
                            break;
                        }
                    }
                }
                Some(_) = config_rx.recv() => {
                    let new_config = Config::load();
                    println!("Config changed, reloading...");
                    if new_config.shortcut != config.shortcut {
                        println!("Shortcut changed to '{}', re-registering...", new_config.shortcut);
                        let _ = proxy.register_shortcut(&new_config.shortcut).await;
                    }
                    config = new_config;
                }
                // Pull audio samples if recording
                Some(bytes) = async {
                    if let Some(state) = &mut recording_state {
                        tokio_stream::StreamExt::next(&mut state.recorder_output.audio_stream).await
                    } else {
                        std::future::pending().await
                    }
                } => {
                    if let Some(state) = &mut recording_state {
                        let chunk: &[f32] = bytemuck::cast_slice(&bytes);
                        state.samples.extend_from_slice(chunk);

                        let speaking = state.vad_processor.process_samples(chunk);
                        if speaking != state.is_speaking {
                            state.is_speaking = speaking;
                            let icon = if speaking {
                                "audio-input-microphone-high-symbolic"
                            } else {
                                "media-record-symbolic"
                            };
                            let color = if speaking {
                                "#00FF00" // Bright green when speaking
                            } else {
                                &state.config.recording_color
                            };
                            let _ = proxy.update(icon, get_menu_items(&history_mgr), "recording", color).await;
                        }
                    }
                }
            }
        }

        // Inner loop exited — wait briefly before reconnecting
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}
