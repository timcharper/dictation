use clap::{Parser, Subcommand};
use libadwaita::prelude::*;
use libadwaita::{Application, ApplicationWindow, PreferencesGroup, ActionRow, PreferencesPage, EntryRow, HeaderBar, ToolbarView};
use gtk4::{glib, Box as GtkBox, Orientation, Button};
use std::sync::{Arc, Mutex};
use tokio::runtime::Runtime;
// use tokio_stream::StreamExt;
use std::env;
use std::path::PathBuf;
use tokio::fs::File;
use tokio_util::io::ReaderStream;
use zbus::Connection;

mod recorder;
mod transcriber;
mod accessibility;
mod config;
mod extension_proxy;
mod audio;
mod mpris;

use config::{Config, BackendConfig, LlmConfig};
use extension_proxy::ExtensionProxy;

const APP_ID: &str = "org.gnome.dictation";

#[derive(Parser)]
#[command(name = "dictation")]
#[command(about = "GNOME Dictation App", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Test microphone by recording to a temporary WAV file
    Microphone {
        /// Duration to record in seconds
        #[arg(short, long, default_value_t = 5)]
        duration: u64,
    },
    /// Test accessibility (AT-SPI) by printing visible text of the active window
    Accessibility,
    /// Transcribe a WAV file using the configured backend
    Transcribe {
        /// Path to the WAV file
        path: PathBuf,
    },
    /// Test GNOME Extension integration
    Extension {
        #[command(subcommand)]
        subcommand: ExtensionCommands,
    },
    /// Test sound playback
    Sound {
        /// Path to the sound file
        path: PathBuf,
    },
    /// Test volume control
    Volume {
        /// Volume level (0.0 to 1.0)
        level: Option<f64>,
    },
    /// Interact with MPRIS media players
    Mpris {
        #[command(subcommand)]
        subcommand: MprisCommands,
    },
    /// Run as a daemon, listening for extension shortcuts
    Daemon,
}

#[derive(Subcommand)]
enum MprisCommands {
    /// Show playback status and metadata for all active MPRIS players
    Status,
    /// Send Play to the active player
    Play,
    /// Send Pause to the active player
    Pause,
}

#[derive(Subcommand)]
enum ExtensionCommands {
    /// Type a string into the focused window
    Type { text: String },
    /// Get clipboard content
    GetClipboard,
    /// Set clipboard content
    SetClipboard { text: String },
    /// Raise the application window
    Raise,
    /// Update tray icon and menu
    UpdateMenu,
    /// Listen for extension events (shortcut, menu selection)
    Listen,
    /// Register a global shortcut
    RegisterShortcut { shortcut: String },
    /// Unregister the global shortcut
    UnregisterShortcut,
    /// Get system volume
    GetVolume,
    /// Set system volume
    SetVolume { level: f64 },
}

fn main() -> glib::ExitCode {
    let args = Cli::parse();

    match args.command {
        Some(Commands::Microphone { duration }) => {
            let rt = Runtime::new().expect("Failed to create Tokio runtime");
            rt.block_on(async move {
                test_microphone(duration).await;
            });
            glib::ExitCode::SUCCESS
        }
        Some(Commands::Accessibility) => {
            let rt = Runtime::new().expect("Failed to create Tokio runtime");
            rt.block_on(async move {
                test_accessibility().await;
            });
            glib::ExitCode::SUCCESS
        }
        Some(Commands::Transcribe { path }) => {
            let rt = Runtime::new().expect("Failed to create Tokio runtime");
            rt.block_on(async move {
                test_transcribe(path).await;
            });
            glib::ExitCode::SUCCESS
        }
        Some(Commands::Extension { subcommand }) => {
            let rt = Runtime::new().expect("Failed to create Tokio runtime");
            rt.block_on(async move {
                test_extension(subcommand).await;
            });
            glib::ExitCode::SUCCESS
        }
        Some(Commands::Sound { path }) => {
            let audio_mgr = audio::AudioManager::new();
            audio_mgr.play_sound(path);
            // Wait for sound to play (sink.detach() means it's background, but for a CLI tool we need to wait)
            std::thread::sleep(std::time::Duration::from_secs(3));
            glib::ExitCode::SUCCESS
        }
        Some(Commands::Volume { level }) => {
            let rt = Runtime::new().expect("Failed to create Tokio runtime");
            rt.block_on(async move {
                let conn = Connection::session().await.expect("Failed to connect to session bus");
                let proxy = ExtensionProxy::new(&conn).await.expect("Failed to create extension proxy");
                if let Some(l) = level {
                    proxy.set_volume(l).await.expect("Failed to set volume");
                    println!("Volume set to {}", l);
                } else {
                    let v = proxy.get_volume().await.expect("Failed to get volume");
                    println!("Current volume: {}", v);
                }
            });
            glib::ExitCode::SUCCESS
        }
        Some(Commands::Mpris { subcommand }) => {
            let rt = Runtime::new().expect("Failed to create Tokio runtime");
            rt.block_on(async move {
                test_mpris(subcommand).await;
            });
            glib::ExitCode::SUCCESS
        }
        Some(Commands::Daemon) => {
            let rt = Runtime::new().expect("Failed to create Tokio runtime");
            rt.block_on(async move {
                run_daemon().await;
            });
            glib::ExitCode::SUCCESS
        }
        None => {
            let app = Application::builder()
                .application_id(APP_ID)
                .build();

            let runtime = Arc::new(Runtime::new().expect("Failed to create Tokio runtime"));

            app.connect_activate(move |app| {
                build_ui(app, runtime.clone());
            });

            app.run()
        }
    }
}

async fn test_mpris(cmd: MprisCommands) {
    let conn = Connection::session().await.expect("Failed to connect to session bus");
    let client = mpris::MprisClient::new(conn);

    let players = client.find_players().await.expect("Failed to list MPRIS players");
    if players.is_empty() {
        println!("No MPRIS players found.");
        return;
    }

    match cmd {
        MprisCommands::Status => {
            for service in &players {
                let proxy = client.get_proxy(service).await.expect("Failed to get player proxy");
                let status = proxy.playback_status().await.unwrap_or_else(|_| "unknown".into());
                let metadata = proxy.metadata().await.unwrap_or_default();
                let title = mpris::string_field(&metadata, "xesam:title");
                let artist = mpris::string_field(&metadata, "xesam:artist");
                let album = mpris::string_field(&metadata, "xesam:album");
                let track_id = mpris::extract_track_id(&metadata);
                println!("Player: {}", service);
                println!("  Status:   {}", status);
                if !title.is_empty()  { println!("  Title:    {}", title); }
                if !artist.is_empty() { println!("  Artist:   {}", artist); }
                if !album.is_empty()  { println!("  Album:    {}", album); }
                println!("  Track ID: {}", track_id);
            }
        }
        MprisCommands::Pause => {
            let service = players.iter()
                .find(|_| true)  // first player; caller picks while something is Playing
                .expect("No players");
            let proxy = client.get_proxy(service).await.expect("Failed to get player proxy");
            proxy.pause().await.expect("Failed to pause");
            println!("Paused: {}", service);
        }
        MprisCommands::Play => {
            let service = players.iter()
                .find(|_| true)
                .expect("No players");
            let proxy = client.get_proxy(service).await.expect("Failed to get player proxy");
            proxy.play().await.expect("Failed to play");
            println!("Playing: {}", service);
        }
    }
}

struct RecordingState {
    samples: Vec<f32>,
    recorder_output: recorder::RecorderOutput,
    original_volume: Option<f64>,
    paused_players: Vec<mpris::PlayerState>,
    config: Config,
}

async fn run_daemon() {
    let conn = Connection::session().await.expect("Failed to connect to session bus");
    let proxy = ExtensionProxy::new(&conn).await.expect("Failed to create extension proxy");
    let audio_mgr = audio::AudioManager::new();
    let mpris_client = mpris::MprisClient::new(conn.clone());

    let config = Config::load();
    println!("Daemon started. Shortcut: {}. Listening for extension signals...", config.shortcut);

    // Initial menu update
    proxy.update("audio-input-microphone-symbolic", vec![
        ("settings", "Settings"),
    ]).await.expect("Failed to update extension menu");

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

                    let url = match &state.config.backend {
                        BackendConfig::WhisperCpp { url } => url.clone(),
                    };
                    let client = transcriber::WhisperClient::new(url);

                    println!("Transcribing...");
                    let stream = tokio_stream::iter(vec![Ok::<bytes::Bytes, std::io::Error>(bytes::Bytes::from(wav_bytes))]);
                    let bytes_only_stream = tokio_stream::StreamExt::filter_map(stream, |res| res.ok());
                    
                    match client.stream_transcription(Box::pin(bytes_only_stream)).await {
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
                } else {
                    println!("Shortcut pressed! Starting recording...");
                    let config = Config::load();
                    
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

async fn test_extension(cmd: ExtensionCommands) {
    let conn = Connection::session().await.expect("Failed to connect to session bus");
    let proxy = ExtensionProxy::new(&conn).await.expect("Failed to create extension proxy");

    match cmd {
        ExtensionCommands::Type { text } => {
            println!("Typing: '{}'", text);
            proxy.type_string(&text).await.expect("Failed to type string");
        }
        ExtensionCommands::GetClipboard => {
            let text = proxy.get_clipboard().await.expect("Failed to get clipboard");
            println!("Clipboard: '{}'", text);
        }
        ExtensionCommands::SetClipboard { text } => {
            println!("Setting clipboard to: '{}'", text);
            proxy.set_clipboard(&text).await.expect("Failed to set clipboard");
        }
        ExtensionCommands::Raise => {
            println!("Raising app...");
            proxy.raise_app().await.expect("Failed to raise app");
        }
        ExtensionCommands::UpdateMenu => {
            println!("Updating menu...");
            proxy.update("audio-input-microphone-symbolic", vec![
                ("test1", "Test Item 1"),
                ("test2", "Test Item 2"),
                ("quit", "Quit"),
            ]).await.expect("Failed to update menu");
        }
        ExtensionCommands::Listen => {
            println!("Listening for extension events. Press Ctrl+C to stop.");
            
            let mut menu_stream = proxy.receive_menu_item_selected().await.expect("Failed to receive menu signals");
            let mut shortcut_stream = proxy.receive_shortcut_pressed().await.expect("Failed to receive shortcut signals");

            loop {
                tokio::select! {
                    Some(signal) = tokio_stream::StreamExt::next(&mut menu_stream) => {
                        let args = signal.args().expect("Failed to parse signal args");
                        println!("Menu Item Selected: {}", args.id);
                    }
                    Some(_) = tokio_stream::StreamExt::next(&mut shortcut_stream) => {
                        println!("Shortcut Pressed!");
                    }
                }
            }
        }
        ExtensionCommands::RegisterShortcut { shortcut } => {
            println!("Registering shortcut: '{}'", shortcut);
            proxy.register_shortcut(&shortcut).await.expect("Failed to register shortcut");
        }
        ExtensionCommands::UnregisterShortcut => {
            println!("Unregistering shortcut...");
            proxy.unregister_shortcut().await.expect("Failed to unregister shortcut");
        }
        ExtensionCommands::GetVolume => {
            let v = proxy.get_volume().await.expect("Failed to get volume");
            println!("Current volume: {}", v);
        }
        ExtensionCommands::SetVolume { level } => {
            proxy.set_volume(level).await.expect("Failed to set volume");
            println!("Volume set to {}", level);
        }
    }
}

async fn test_microphone(duration_secs: u64) {
    println!("Recording for {} seconds...", duration_secs);
    
    let recorder = recorder::AudioRecorder::new();
    let mut output = recorder.start_recording();
    
    let mut samples: Vec<f32> = Vec::new();
    let start_time = std::time::Instant::now();
    
    while start_time.elapsed().as_secs() < duration_secs {
        if let Some(bytes) = tokio_stream::StreamExt::next(&mut output.audio_stream).await {
            let chunk: &[f32] = bytemuck::cast_slice(&bytes);
            samples.extend_from_slice(chunk);
        }
    }

    drop(output.stream);

    let temp_dir = env::temp_dir();
    let file_path = temp_dir.join("dictation_test.wav");
    
    let spec = hound::WavSpec {
        channels: output.config.channels as u16,
        sample_rate: output.config.sample_rate.0,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };

    let mut writer = hound::WavWriter::create(&file_path, spec).expect("Failed to create WAV writer");
    for sample in samples {
        writer.write_sample(sample).expect("Failed to write sample");
    }
    writer.finalize().expect("Failed to finalize WAV file");

    println!("Success! Recorded to: {:?}", file_path);
}

async fn test_transcribe(path: PathBuf) {
    println!("Transcribing file: {:?}", path);
    
    let config = Config::load();
    let url = match config.backend {
        BackendConfig::WhisperCpp { url } => url,
    };
    
    println!("Using backend: WhisperCpp at {}", url);
    let client = transcriber::WhisperClient::new(url);
    
    let file = File::open(path).await.expect("Failed to open WAV file");
    let stream = ReaderStream::new(file);
    
    // Map Result<Bytes, io::Error> to Bytes, ignoring errors for now
    let bytes_stream = tokio_stream::StreamExt::filter_map(stream, |res| res.ok());
    
    let mut transcription_stream = match client.stream_transcription(Box::pin(bytes_stream)).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to start transcription stream: {:?}", e);
            return;
        }
    };

    while let Some(res) = futures_util::StreamExt::next(&mut transcription_stream).await {
        match res {
            Ok(resp) => {
                if !resp.text.is_empty() {
                    println!("Transcription: {}", resp.text);
                }
            },
            Err(e) => eprintln!("Transcription error: {:?}", e),
        }
    }
}

async fn test_accessibility() {
    println!("Initializing AT-SPI...");
    let manager = match accessibility::AccessibilityManager::new().await {
        Ok(m) => m,
        Err(e) => {
            eprintln!("Failed to initialize AT-SPI: {:?}", e);
            return;
        }
    };

    println!("\nGrabbing visible text of active window...");
    match manager.get_visible_text().await {
        Ok(texts) => {
            for (i, text) in texts.iter().enumerate() {
                println!("VISIBLE {}: \"{}\"", i + 1, text);
            }
        }
        Err(e) => eprintln!("Error getting visible text: {:?}", e),
    }

    println!("\nGrabbing focused context (ancestors included)...");
    match manager.get_focused_context().await {
        Ok(texts) => {
            if texts.is_empty() {
                println!("No focused context found.");
            } else {
                for (i, text) in texts.iter().enumerate() {
                    println!("CONTEXT {}: \"{}\"", i + 1, text);
                }
            }
        }
        Err(e) => eprintln!("Error getting focused context: {:?}", e),
    }
}

fn build_ui(app: &Application, runtime: Arc<Runtime>) {
    let config = Arc::new(Mutex::new(Config::load()));

    let window = ApplicationWindow::builder()
        .application(app)
        .title("Dictation Settings")
        .default_width(600)
        .default_height(450)
        .build();

    let toolbar_view = ToolbarView::new();
    let header_bar = HeaderBar::new();
    toolbar_view.add_top_bar(&header_bar);

    let content_vbox = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();

    let page = PreferencesPage::new();
    let general_group = PreferencesGroup::builder()
        .title("General")
        .description("Basic configuration for Dictation")
        .build();

    let mic_row = ActionRow::builder()
        .title("Microphone Device")
        .subtitle("default")
        .build();
    general_group.add(&mic_row);

    let initial_shortcut = {
        let cfg = config.lock().unwrap();
        cfg.shortcut.clone()
    };

    let shortcut_row = ActionRow::builder()
        .title("Shortcut")
        .subtitle(glib::markup_escape_text(&initial_shortcut))
        .build();

    let edit_button = Button::builder()
        .label("Edit")
        .valign(gtk4::Align::Center)
        .build();

    let clear_button = Button::builder()
        .icon_name("edit-clear-symbolic")
        .valign(gtk4::Align::Center)
        .has_frame(false)
        .build();

    shortcut_row.add_suffix(&edit_button);
    shortcut_row.add_suffix(&clear_button);

    let config_clone = config.clone();
    let runtime_clone = runtime.clone();
    let shortcut_row_clone = shortcut_row.clone();
    clear_button.connect_clicked(move |_| {
        shortcut_row_clone.set_subtitle("");
        let mut cfg = config_clone.lock().unwrap();
        cfg.shortcut = "".to_string();
        cfg.save();

        let rt = runtime_clone.clone();
        rt.spawn(async move {
            let conn = Connection::session().await.ok();
            if let Some(c) = conn {
                if let Ok(proxy) = ExtensionProxy::new(&c).await {
                    let _ = proxy.unregister_shortcut().await;
                }
            }
        });
    });

    let config_clone = config.clone();
    let runtime_clone = runtime.clone();
    let shortcut_row_clone = shortcut_row.clone();
    let window_clone = window.clone();
    edit_button.connect_clicked(move |_| {
        let config = config_clone.clone();
        let runtime = runtime_clone.clone();
        let shortcut_row = shortcut_row_clone.clone();
        
        // Unregister current shortcut while recording
        let rt = runtime.clone();
        rt.spawn(async move {
            let conn = Connection::session().await.ok();
            if let Some(c) = conn {
                if let Ok(proxy) = ExtensionProxy::new(&c).await {
                    let _ = proxy.unregister_shortcut().await;
                }
            }
        });

        // Create recording modal
        let dialog = gtk4::Window::builder()
            .title("Record Shortcut")
            .default_width(300)
            .default_height(150)
            .modal(true)
            .transient_for(&window_clone)
            .build();

        let vbox = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(12)
            .margin_top(24)
            .margin_bottom(24)
            .margin_start(24)
            .margin_end(24)
            .build();

        let label = gtk4::Label::new(Some("Press the key combination you want to use"));
        vbox.append(&label);

        let status_label = gtk4::Label::new(Some("Listening..."));
        vbox.append(&status_label);

        let cancel_button = Button::with_label("Cancel");
        vbox.append(&cancel_button);

        dialog.set_child(Some(&vbox));

        let key_controller = gtk4::EventControllerKey::new();
        let dialog_clone = dialog.clone();
        let config_clone = config.clone();
        let runtime_clone = runtime.clone();
        let shortcut_row_clone = shortcut_row.clone();
        
        key_controller.connect_key_pressed(move |_controller, keyval, _keycode, state| {
            let modifiers = state & (gtk4::gdk::ModifierType::CONTROL_MASK | 
                                    gtk4::gdk::ModifierType::ALT_MASK | 
                                    gtk4::gdk::ModifierType::SHIFT_MASK | 
                                    gtk4::gdk::ModifierType::SUPER_MASK);
            if modifiers.is_empty() && keyval.name().map(|n| n.starts_with("F")).unwrap_or(false) == false {
                // Ignore plain keys that aren't function keys
                return glib::Propagation::Proceed;
            }

            let mut accel = String::new();
            if state.contains(gtk4::gdk::ModifierType::CONTROL_MASK) { accel.push_str("<Control>"); }
            if state.contains(gtk4::gdk::ModifierType::ALT_MASK) { accel.push_str("<Alt>"); }
            if state.contains(gtk4::gdk::ModifierType::SHIFT_MASK) { accel.push_str("<Shift>"); }
            if state.contains(gtk4::gdk::ModifierType::SUPER_MASK) { accel.push_str("<Super>"); }

            if let Some(name) = keyval.name() {
                // Map GDK names to what GNOME expects (simplified)
                let name = match name.as_str() {
                    "Control_L" | "Control_R" | "Alt_L" | "Alt_R" | "Shift_L" | "Shift_R" | "Super_L" | "Super_R" => return glib::Propagation::Proceed,
                    n => n,
                };
                accel.push_str(name);
            }

            if !accel.is_empty() {
                let mut cfg = config_clone.lock().unwrap();
                cfg.shortcut = accel.clone();
                cfg.save();
                shortcut_row_clone.set_subtitle(&glib::markup_escape_text(&accel));

                let rt = runtime_clone.clone();
                let accel_to_reg = accel.clone();
                rt.spawn(async move {
                    let conn = Connection::session().await.ok();
                    if let Some(c) = conn {
                        if let Ok(proxy) = ExtensionProxy::new(&c).await {
                            let _ = proxy.register_shortcut(&accel_to_reg).await;
                        }
                    }
                });

                dialog_clone.close();
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });

        dialog.add_controller(key_controller);
        
        let dialog_cancel = dialog.clone();
        let runtime_cancel = runtime.clone();
        let config_cancel = config.clone();
        cancel_button.connect_clicked(move |_| {
            // Restore previous shortcut if cancelled
            let prev_accel = config_cancel.lock().unwrap().shortcut.clone();
            let rt = runtime_cancel.clone();
            rt.spawn(async move {
                if !prev_accel.is_empty() {
                    let conn = Connection::session().await.ok();
                    if let Some(c) = conn {
                        if let Ok(proxy) = ExtensionProxy::new(&c).await {
                            let _ = proxy.register_shortcut(&prev_accel).await;
                        }
                    }
                }
            });
            dialog_cancel.close();
        });

        dialog.present();
    });
    general_group.add(&shortcut_row);

    let initial_delay = {
        let cfg = config.lock().unwrap();
        cfg.typing_delay_ms
    };

    let delay_row = EntryRow::builder()
        .title("Typing Delay (ms)")
        .text(&initial_delay.to_string())
        .build();

    let config_clone = config.clone();
    delay_row.connect_text_notify(move |row| {
        if let Ok(val) = row.text().to_string().parse::<u64>() {
            let mut cfg = config_clone.lock().unwrap();
            cfg.typing_delay_ms = val;
            cfg.save();
        }
    });
    general_group.add(&delay_row);

    // Whisper Server URL
    let whisper_group = PreferencesGroup::builder()
        .title("Transcription (Whisper)")
        .build();
    
    let initial_whisper_url = {
        let cfg = config.lock().unwrap();
        match &cfg.backend {
            BackendConfig::WhisperCpp { url } => url.clone(),
        }
    };

    let whisper_url_row = EntryRow::builder()
        .title("Whisper Server URL")
        .text(&initial_whisper_url)
        .build();
    
    let config_clone = config.clone();
    whisper_url_row.connect_text_notify(move |row| {
        let mut cfg = config_clone.lock().unwrap();
        cfg.backend = BackendConfig::WhisperCpp {
            url: row.text().to_string(),
        };
        cfg.save();
    });

    whisper_group.add(&whisper_url_row);

    // LLM Provider (Ollama)
    let llm_group = PreferencesGroup::builder()
        .title("LLM Provider (Ollama)")
        .build();
    
    let (initial_ollama_url, initial_ollama_model) = {
        let cfg = config.lock().unwrap();
        match &cfg.llm {
            LlmConfig::Ollama { url, model } => (url.clone(), model.clone()),
        }
    };

    let ollama_url_row = EntryRow::builder()
        .title("Ollama Server URL")
        .text(&initial_ollama_url)
        .build();
    
    let config_clone = config.clone();
    ollama_url_row.connect_text_notify(move |row| {
        let mut cfg = config_clone.lock().unwrap();
        let model = match &cfg.llm {
            LlmConfig::Ollama { model, .. } => model.clone(),
        };
        cfg.llm = LlmConfig::Ollama {
            url: row.text().to_string(),
            model,
        };
        cfg.save();
    });

    let ollama_model_row = EntryRow::builder()
        .title("Ollama Model")
        .text(&initial_ollama_model)
        .build();
    
    let config_clone = config.clone();
    ollama_model_row.connect_text_notify(move |row| {
        let mut cfg = config_clone.lock().unwrap();
        let url = match &cfg.llm {
            LlmConfig::Ollama { url, .. } => url.clone(),
        };
        cfg.llm = LlmConfig::Ollama {
            url,
            model: row.text().to_string(),
        };
        cfg.save();
    });

    llm_group.add(&ollama_url_row);
    llm_group.add(&ollama_model_row);

    // Sound settings
    let sound_group = PreferencesGroup::builder()
        .title("Sound &amp; Volume")
        .build();

    let (initial_start_sound, initial_end_sound, initial_ducking_volume) = {
        let cfg = config.lock().unwrap();
        (
            cfg.sound.start_sound.clone().unwrap_or_default(),
            cfg.sound.end_sound.clone().unwrap_or_default(),
            cfg.sound.ducking_volume,
        )
    };

    let start_sound_row = EntryRow::builder()
        .title("Start Sound (Path)")
        .text(&initial_start_sound)
        .build();
    let config_clone = config.clone();
    start_sound_row.connect_text_notify(move |row| {
        let mut cfg = config_clone.lock().unwrap();
        let text = row.text().to_string();
        cfg.sound.start_sound = if text.is_empty() { None } else { Some(text) };
        cfg.save();
    });

    let end_sound_row = EntryRow::builder()
        .title("End Sound (Path)")
        .text(&initial_end_sound)
        .build();
    let config_clone = config.clone();
    end_sound_row.connect_text_notify(move |row| {
        let mut cfg = config_clone.lock().unwrap();
        let text = row.text().to_string();
        cfg.sound.end_sound = if text.is_empty() { None } else { Some(text) };
        cfg.save();
    });

    let ducking_row = EntryRow::builder()
        .title("Ducking Volume (0.0 - 1.0)")
        .text(&initial_ducking_volume.to_string())
        .build();
    let config_clone = config.clone();
    ducking_row.connect_text_notify(move |row| {
        if let Ok(val) = row.text().to_string().parse::<f32>() {
            let mut cfg = config_clone.lock().unwrap();
            cfg.sound.ducking_volume = val.clamp(0.0, 1.0);
            cfg.save();
        }
    });

    sound_group.add(&start_sound_row);
    sound_group.add(&end_sound_row);
    sound_group.add(&ducking_row);

    page.add(&general_group);
    page.add(&whisper_group);
    page.add(&llm_group);
    page.add(&sound_group);

    content_vbox.append(&page);

    let start_button = Button::builder()
        .label("Start Recording (Test Stream)")
        .css_classes(vec!["suggested-action"])
        .build();

    let rt_clone = runtime.clone();
    let config_for_recording = config.clone();
    start_button.connect_clicked(move |_| {
        let rt = rt_clone.clone();
        let config = config_for_recording.clone();
        rt.spawn(async move {
            let recorder = recorder::AudioRecorder::new();
            let recorder::RecorderOutput { stream, audio_stream, .. } = recorder.start_recording();
            
            let url = {
                let cfg = config.lock().unwrap();
                match &cfg.backend {
                    BackendConfig::WhisperCpp { url } => url.clone(),
                }
            };
            
            let client = transcriber::WhisperClient::new(url);
            
            // To satisfy Send bound, we MUST NOT hold 'stream' across await.
            // But we need it to keep recording. 
            // Since this is just a TEST button, we'll forget the stream so it keeps running.
            std::mem::forget(stream);

            let mut transcription_stream = match client.stream_transcription(Box::pin(audio_stream)).await {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Failed to start transcription stream: {:?}", e);
                    return;
                }
            };

            while let Some(res) = futures_util::StreamExt::next(&mut transcription_stream).await {
                match res {
                    Ok(resp) => println!("Transcription: {} (final: {})", resp.text, resp.is_final),
                    Err(e) => eprintln!("Transcription error: {:?}", e),
                }
            }
        });
    });

    content_vbox.append(&start_button);
    
    toolbar_view.set_content(Some(&content_vbox));
    window.set_content(Some(&toolbar_view));
    window.present();
}
