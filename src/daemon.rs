use std::time::Duration;
use std::pin::Pin;
use zbus::Connection;
use zbus::fdo::DBusProxy;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::ReceiverStream;
use bytes::Bytes;

use crate::audio;
use crate::config::Config;
use crate::extension_proxy::ExtensionProxy;
use crate::mpris;
use crate::recorder;
use crate::transcriber_factory::create_transcriber;
use crate::traits::TranscriptionResult;
use crate::vad::VadProcessor;
use crate::accessibility::AccessibilityManager;

/// Ensures audio volume and MPRIS state are restored when recording ends or the daemon encounters an error.
struct RecordingGuard {
    original_volume: Option<f64>,
    paused_players: Vec<mpris::PlayerState>,
    mpris_client: mpris::MprisClient,
    conn: Connection,
    sound_config: crate::config::SoundConfig,
}

impl RecordingGuard {
    fn new(
        original_volume: Option<f64>,
        paused_players: Vec<mpris::PlayerState>,
        mpris_client: &mpris::MprisClient,
        conn: &Connection,
        sound_config: &crate::config::SoundConfig,
    ) -> Self {
        Self {
            original_volume,
            paused_players,
            mpris_client: mpris_client.clone(),
            conn: conn.clone(),
            sound_config: sound_config.clone(),
        }
    }
}

impl Drop for RecordingGuard {
    fn drop(&mut self) {
        let original_volume = self.original_volume;
        let paused_players = std::mem::take(&mut self.paused_players);
        let audio_mgr = audio::AudioManager::new();
        let mpris_client = self.mpris_client.clone();
        let conn = self.conn.clone();
        let sound_config = self.sound_config.clone();

        tokio::spawn(async move {
            println!("[DEBUG] RecordingGuard restoring state...");
            if let Ok(proxy) = ExtensionProxy::new(&conn).await {
                // Restore volume and play end sound
                audio_mgr.restore_and_play_end(&proxy, &sound_config, original_volume).await;
            }

            // Resume MPRIS players
            for p_state in paused_players {
                if let Ok(player_proxy) = mpris_client.get_proxy(&p_state.service).await {
                    if let Ok(status) = player_proxy.playback_status().await {
                        let status: String = status;
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
        });
    }
}

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
    recorder_output: recorder::RecorderOutput,
    original_volume: Option<f64>,
    paused_players: Vec<mpris::PlayerState>,
    config: Config,
    vad_processor: VadProcessor,
    is_speaking: bool,
    cursor_context: Option<String>,
    audio_tx: tokio::sync::mpsc::Sender<Bytes>,
    transcription_stream: Pin<Box<dyn tokio_stream::Stream<Item = Result<TranscriptionResult, String>> + Send>>,
    full_text: String,
}

struct DaemonServer {
    trigger_tx: tokio::sync::mpsc::Sender<()>,
}

#[zbus::interface(name = "com.timcharper.dictation.Daemon")]
impl DaemonServer {
    async fn trigger(&self) {
        let _ = self.trigger_tx.send(()).await;
    }
}

enum RecordingEvent {
    Audio(Vec<f32>),
    Transcription(Result<TranscriptionResult, String>),
}

pub async fn run_daemon() {
    let audio_mgr = audio::AudioManager::new();
    let mut history_mgr = crate::history::HistoryManager::load();

    'connection: loop {
        let conn = match Connection::session().await {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Failed to connect to session bus: {e}. Retrying in 10s...");
                tokio::time::sleep(Duration::from_secs(10)).await;
                continue 'connection;
            }
        };

        if let Err(e) = conn.request_name("com.timcharper.dictation.Daemon").await {
            eprintln!("Failed to request daemon name: {e}. Retrying in 10s...");
            tokio::time::sleep(Duration::from_secs(10)).await;
            continue 'connection;
        }

        let (trigger_tx, mut trigger_rx) = tokio::sync::mpsc::channel::<()>(1);
        let daemon_server = DaemonServer { trigger_tx };
        conn.object_server()
            .at("/com/timcharper/dictation/Daemon", daemon_server)
            .await
            .expect("Failed to export Daemon D-Bus object");

        let mpris_client = mpris::MprisClient::new(conn.clone());
        let accessibility_mgr = tokio::time::timeout(
            Duration::from_secs(2),
            AccessibilityManager::new()
        ).await.ok().and_then(|res| res.ok());

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

        let mut name_owner_stream = match DBusProxy::new(&conn).await {
            Ok(p) => match p.receive_name_owner_changed().await {
                Ok(s) => Some(s),
                Err(e) => { eprintln!("Warning: could not subscribe to NameOwnerChanged: {e}"); None }
            },
            Err(e) => { eprintln!("Warning: could not connect to org.freedesktop.DBus: {e}"); None }
        };

        let mut retry_delay = Duration::from_secs(2);

        'retry: loop {
            let proxy = match ExtensionProxy::new(&conn).await {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("Extension not available: {e}. Retrying in {}s...", retry_delay.as_secs());
                    tokio::time::sleep(retry_delay).await;
                    retry_delay = (retry_delay * 2).min(Duration::from_secs(30));
                    if retry_delay >= Duration::from_secs(30) {
                        eprintln!("Max retry delay reached, recreating D-Bus connection...");
                        continue 'connection;
                    }
                    continue 'retry;
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
                eprintln!("Failed initial extension update: {e}. Retrying in {}s...", retry_delay.as_secs());
                tokio::time::sleep(retry_delay).await;
                retry_delay = (retry_delay * 2).min(Duration::from_secs(30));
                if retry_delay >= Duration::from_secs(30) {
                    eprintln!("Max retry delay reached, recreating D-Bus connection...");
                    continue 'connection;
                }
                continue 'retry;
            }

            if let Err(e) = proxy.register_shortcut(&config.shortcut).await {
                eprintln!("Failed to register shortcut: {e}. Retrying in {}s...", retry_delay.as_secs());
                tokio::time::sleep(retry_delay).await;
                retry_delay = (retry_delay * 2).min(Duration::from_secs(30));
                if retry_delay >= Duration::from_secs(30) {
                    eprintln!("Max retry delay reached, recreating D-Bus connection...");
                    continue 'connection;
                }
                continue 'retry;
            }
            
            // Reset delay on success
            retry_delay = Duration::from_secs(2);

            let mut menu_stream = match proxy.receive_menu_item_selected().await {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Failed to subscribe to menu signals: {e}. Retrying in 2s...");
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    continue 'retry;
                }
            };

            let mut shortcut_stream = match proxy.receive_shortcut_pressed().await {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Failed to subscribe to shortcut signals: {e}. Retrying in 2s...");
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    continue 'retry;
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
                        if let Ok(Ok(cls)) = tokio::time::timeout(Duration::from_millis(200), proxy.get_focused_window_class()).await {
                            wm_at_last_focus = Some(cls);
                        }
                    }
                    sleep_signal = async {
                        if let Some(s) = &mut sleep_stream {
                            s.next().await
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
                                    break 'retry;
                                }
                            }
                        } else {
                            eprintln!("Sleep event stream ended unexpectedly.");
                            // Don't break — keep running without sleep detection
                            sleep_stream = None;
                        }
                    }
                    name_owner_signal = async {
                        if let Some(s) = &mut name_owner_stream {
                            s.next().await
                        } else {
                            std::future::pending::<Option<_>>().await
                        }
                    } => {
                        match name_owner_signal {
                            Some(sig) => {
                                if let Ok(args) = sig.args() {
                                    if args.name == "com.timcharper.dictation.Extension" && args.new_owner.is_none() {
                                        println!("Extension bus name disappeared (GNOME Shell restart?). Reconnecting...");
                                        break 'retry;
                                    }
                                }
                            }
                            None => {
                                eprintln!("Name owner stream ended. Reconnecting...");
                                break 'retry;
                            }
                        }
                    }
                    menu_signal = menu_stream.next() => {
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
                                break 'retry;
                            }
                        }
                    }
                    _trigger = async {
                        tokio::select! {
                            res = shortcut_stream.next() => res.map(|_| ()),
                            res = trigger_rx.recv() => res.map(|_| ()),
                        }
                    } => {
                        match _trigger {
                            Some(_) => {
                                if let Some(mut state) = recording_state.take() {
                                    println!("Trigger received! Stopping recording and transcribing...");
                                    let stop_time = std::time::Instant::now();

                                    // Signal transcribing state
                                    let _ = proxy.update("emblem-synchronizing-symbolic", get_menu_items(&history_mgr), "transcribing", &state.config.transcribing_color).await;

                                    // The guard will handle restoration on drop (at the end of this block)
                                    let _guard = RecordingGuard::new(
                                        state.original_volume,
                                        std::mem::take(&mut state.paused_players),
                                        &mpris_client,
                                        &conn,
                                        &state.config.sound,
                                    );

                                    // Close the audio stream
                                    drop(state.audio_tx);

                                    println!("Transcribing...");
                                    let mut full_text = state.full_text;
                                    while let Some(res) = state.transcription_stream.next().await {
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

                                    println!("Dictation cycle complete.");
                                    let _ = proxy.update("audio-input-microphone-symbolic", get_menu_items(&history_mgr), "idle", "").await;
                                } else {
                                    println!("Shortcut pressed! Starting recording...");
                                    let config = Config::load();

                                    println!("[DEBUG] Updating extension to recording state...");
                                    let _ = tokio::time::timeout(
                                        Duration::from_millis(500),
                                        proxy.update("media-record-symbolic", get_menu_items(&history_mgr), "recording", &config.recording_color)
                                    ).await;

                                    // 1. MPRIS Pause
                                    println!("[DEBUG] Pausing MPRIS players...");
                                    let mut paused_players = Vec::new();
                                    if let Ok(Ok(players)) = tokio::time::timeout(Duration::from_millis(500), mpris_client.find_players()).await {
                                        for service in players {
                                            if let Ok(Ok(player_proxy)) = tokio::time::timeout(Duration::from_millis(200), mpris_client.get_proxy(&service)).await {
                                                if let Ok(Ok(status)) = tokio::time::timeout(Duration::from_millis(200), player_proxy.playback_status()).await {
                                                    if status == "Playing" {
                                                        if let Ok(Ok(metadata)) = tokio::time::timeout(Duration::from_millis(200), player_proxy.metadata()).await {
                                                            let track_id = mpris::extract_track_id(&metadata);
                                                            println!("Pausing player: {} (track: {})", service, track_id);
                                                            let _ = tokio::time::timeout(Duration::from_millis(200), player_proxy.pause()).await;
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
                                    println!("[DEBUG] Ducking volume and playing start sound...");
                                    let original_volume = tokio::time::timeout(
                                        Duration::from_secs(1),
                                        audio_mgr.duck_and_play_start(&proxy, &config.sound)
                                    ).await.unwrap_or(None);

                                    let mut cursor_context = None;
                                    if let Some(mgr) = &accessibility_mgr {
                                        println!("[DEBUG] Capturing AT-SPI context...");
                                        let current_wm = match tokio::time::timeout(Duration::from_millis(500), proxy.get_focused_window_class()).await {
                                            Ok(Ok(cls)) => cls,
                                            _ => String::new(),
                                        };

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
                                        
                                        if !is_stale && !is_blacklisted {
                                            println!("[DEBUG] Fetching cursor info from AT-SPI...");
                                            match tokio::time::timeout(Duration::from_secs(1), mgr.get_cursor_info()).await {
                                                Ok(Ok(Some(info))) => {
                                                    println!("[DEBUG] AT-SPI context captured ({} chars)", info.text_before.len());
                                                    cursor_context = Some(info.text_before);
                                                },
                                                Ok(Ok(None)) => println!("[DEBUG] AT-SPI: No cursor found"),
                                                Ok(Err(e)) => eprintln!("[AT-SPI] get_cursor_info error: {e}"),
                                                Err(_) => eprintln!("[AT-SPI] get_cursor_info timed out"),
                                            }
                                        }
                                    }

                                    // 3. Start Recorder
                                    println!("[DEBUG] Initializing audio recorder...");
                                    let recorder = recorder::AudioRecorder::new();
                                    let output = recorder.start_recording();
                                    
                                    println!("[DEBUG] Setting up VAD processor...");
                                    let vad_processor = VadProcessor::new(
                                        output.config.sample_rate.0,
                                        output.config.channels as u16,
                                    );

                                    // 4. Initialize Transcriber and Audio Stream
                                    let (audio_tx, audio_rx) = tokio::sync::mpsc::channel::<Bytes>(100);
                                    let transcriber = create_transcriber(&config.backend, cursor_context.clone());
                                    let transcription_stream = match transcriber.stream_transcription(
                                        crate::traits::AudioFormat { sample_rate: 16000, channels: 1 },
                                        Box::pin(ReceiverStream::new(audio_rx))
                                    ).await {
                                        Ok(s) => s,
                                        Err(e) => {
                                            eprintln!("Failed to start transcription: {}", e);
                                            // Still start recording so we don't crash, but it won't work
                                            Box::pin(tokio_stream::iter(vec![Err(e)]))
                                        }
                                    };

                                    recording_state = Some(RecordingState {
                                        recorder_output: output,
                                        original_volume,
                                        paused_players,
                                        config,
                                        vad_processor,
                                        is_speaking: false,
                                        cursor_context,
                                        audio_tx,
                                        transcription_stream,
                                        full_text: String::new(),
                                    });
                                    println!("[DEBUG] Recording started successfully.");
                                }
                            }
                            None => {
                                eprintln!("Shortcut stream ended; extension disconnected. Reconnecting...");
                                break 'retry;
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
                    // Poll audio samples and transcription results if recording
                    Some(event) = async {
                        if let Some(state) = &mut recording_state {
                            tokio::select! {
                                biased;
                                Some(chunk) = state.recorder_output.audio_stream.next() => Some(RecordingEvent::Audio(chunk)),
                                Some(res) = state.transcription_stream.next() => Some(RecordingEvent::Transcription(res)),
                                else => None,
                            }
                        } else {
                            std::future::pending().await
                        }
                    } => {
                        if let Some(state) = &mut recording_state {
                            match event {
                                RecordingEvent::Audio(samples_chunk) => {
                                    let speaking = state.vad_processor.process_samples(&samples_chunk);
                                    
                                    // Stream processed samples to transcriber
                                    let processed_samples = state.vad_processor.take_transcription_samples();
                                    if !processed_samples.is_empty() {
                                        let raw_bytes = Bytes::from(bytemuck::cast_slice::<f32, u8>(&processed_samples).to_vec());
                                        if let Err(e) = state.audio_tx.try_send(raw_bytes) {
                                            eprintln!("Warning: Audio channel full, dropping samples: \"{:?}\"", e);
                                        }
                                    }

                                    if speaking != state.is_speaking {
                                        state.is_speaking = speaking;
                                        let icon = if speaking {
                                            "audio-input-microphone-high-symbolic"
                                        } else {
                                            "media-record-symbolic"
                                        };
                                        let color = if speaking {
                                            "#00FF00".to_string() // Bright green when speaking
                                        } else {
                                            state.config.recording_color.clone()
                                        };
                                        
                                        let proxy_clone = proxy.clone();
                                        let menu_items = get_menu_items(&history_mgr);
                                        tokio::spawn(async move {
                                            let _ = proxy_clone.update(icon, menu_items, "recording", &color).await;
                                        });
                                    }
                                }
                                RecordingEvent::Transcription(transcription_res) => {
                                    match transcription_res {
                                        Ok(resp) => {
                                            if resp.is_final {
                                                state.full_text = resp.text;
                                            } else if !resp.text.is_empty() {
                                                state.full_text = resp.text;
                                            }
                                        }
                                        Err(e) => eprintln!("Transcription error during recording: {}", e),
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Inner loop exited — wait briefly before reconnecting
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}
