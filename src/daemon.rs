use zbus::Connection;

use crate::audio;
use crate::config::Config;
use crate::extension_proxy::ExtensionProxy;
use crate::mpris;
use crate::recorder;
use crate::transcriber_factory::create_transcriber;

struct RecordingState {
    samples: Vec<f32>,
    recorder_output: recorder::RecorderOutput,
    original_volume: Option<f64>,
    paused_players: Vec<mpris::PlayerState>,
    config: Config,
}

pub async fn run_daemon() {
    let conn = Connection::session().await.expect("Failed to connect to session bus");
    
    // Own a name so the extension can track our presence
    conn.request_name("org.gnome.dictation.Daemon")
        .await
        .expect("Failed to request daemon name. Is another instance running?");

    let proxy = ExtensionProxy::new(&conn).await.expect("Failed to create extension proxy");
    let audio_mgr = audio::AudioManager::new();
    let mpris_client = mpris::MprisClient::new(conn.clone());

    let config = Config::load();
    println!("Daemon started. Shortcut: {}. Listening for extension signals...", config.shortcut);

    // Initial menu update
    proxy.update("audio-input-microphone-symbolic", vec![
        ("settings", "Settings"),
    ], "idle", "").await.expect("Failed to update extension menu");

    // Register shortcut from config
    proxy.register_shortcut(&config.shortcut).await.expect("Failed to register shortcut");

    let mut menu_stream = proxy.receive_menu_item_selected().await.expect("Failed to receive menu signals");
    let mut shortcut_stream = proxy.receive_shortcut_pressed().await.expect("Failed to receive shortcut signals");

    let mut recording_state: Option<RecordingState> = None;

    loop {
        tokio::select! {
            Some(signal) = tokio_stream::StreamExt::next(&mut menu_stream) => {
                let args = signal.args().expect("Failed to parse signal args");
                if args.id == "settings" {
                    println!("Opening settings dialog...");
                    let current_exe = std::env::current_exe().expect("Failed to get current exe");
                    let _ = std::process::Command::new(current_exe).spawn();
                }
            }
            Some(_) = tokio_stream::StreamExt::next(&mut shortcut_stream) => {
                if let Some(state) = recording_state.take() {
                    println!("Shortcut pressed! Stopping recording and transcribing...");
                    let stop_time = std::time::Instant::now();

                    // Signal transcribing state
                    let _ = proxy.update("emblem-synchronizing-symbolic", vec![("settings", "Settings")], "transcribing", &state.config.transcribing_color).await;

                    // Restore volume and play end sound immediately
                    audio_mgr.restore_and_play_end(&proxy, &state.config.sound, state.original_volume).await;
                    
                    // Create a WAV in memory to send to whisper
                    let spec = hound::WavSpec {
                        channels: state.recorder_output.config.channels as u16,
                        sample_rate: state.recorder_output.config.sample_rate.0,
                        bits_per_sample: 32,
                        sample_format: hound::SampleFormat::Float,
                    };
                    
                    let mut wav_data = std::io::Cursor::new(Vec::new());
                    {
                        let mut writer = hound::WavWriter::new(&mut wav_data, spec).expect("Failed to create WAV writer");
                        for sample in state.samples {
                            writer.write_sample(sample).expect("Failed to write sample");
                        }
                        writer.finalize().expect("Failed to finalize WAV");
                    }
                    let wav_bytes = wav_data.into_inner();

                    let transcriber = create_transcriber(&state.config.backend);

                    println!("Transcribing...");
                    let stream = tokio_stream::iter(vec![Ok::<bytes::Bytes, std::io::Error>(bytes::Bytes::from(wav_bytes))]);
                    let bytes_only_stream = tokio_stream::StreamExt::filter_map(stream, |res| res.ok());
                    
                    match transcriber.stream_transcription(Box::pin(bytes_only_stream)).await {
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
                                let _ = proxy.type_string(&full_text).await;
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
                    let _ = proxy.update("audio-input-microphone-symbolic", vec![("settings", "Settings")], "idle", "").await;
                } else {
                    println!("Shortcut pressed! Starting recording...");
                    let config = Config::load();
                    
                    let _ = proxy.update("media-record-symbolic", vec![("settings", "Settings")], "recording", &config.recording_color).await;

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
                    
                    recording_state = Some(RecordingState {
                        samples: Vec::new(),
                        recorder_output: output,
                        original_volume,
                        paused_players,
                        config,
                    });
                }
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
                }
            }
        }
    }
}
