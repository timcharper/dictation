use clap::{Parser, Subcommand};
use libadwaita::prelude::*;
use libadwaita::Application;
use gtk4::glib;
use std::sync::Arc;
use tokio::runtime::Runtime;
use std::env;
use std::path::PathBuf;
use tokio::fs::File;
use tokio_util::io::ReaderStream;
use zbus::Connection;

mod recorder;
mod accessibility;
mod config;
mod extension_proxy;
mod audio;
mod mpris;
mod traits;
mod transcriber_whisper;
mod transcriber_factory;
mod ui;
mod daemon;

use config::Config;
use extension_proxy::ExtensionProxy;
use transcriber_factory::create_transcriber;

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
            std::thread::sleep(std::time::Duration::from_secs(3));
            glib::ExitCode::SUCCESS
        }
        Some(Commands::Volume { level }) => {
            let audio_mgr = audio::AudioManager::new();
            if let Some(l) = level {
                audio_mgr.set_volume(l);
                println!("Volume set to {}", l);
            } else {
                if let Some(v) = audio_mgr.get_volume() {
                    println!("Current volume: {:.2}", v);
                } else {
                    println!("Failed to get volume");
                }
            }
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
                daemon::run_daemon().await;
            });
            glib::ExitCode::SUCCESS
        }
        None => {
            let app = Application::builder()
                .application_id(APP_ID)
                .build();

            let runtime = Arc::new(Runtime::new().expect("Failed to create Tokio runtime"));

            app.connect_activate(move |app| {
                ui::build_ui(app, runtime.clone());
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
                .find(|_| true)
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
            ], "idle", "").await.expect("Failed to update menu");
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
    let transcriber = create_transcriber(&config.backend);
    
    println!("Using configured backend");
    
    let file = File::open(path).await.expect("Failed to open WAV file");
    let stream = ReaderStream::new(file);
    
    let bytes_stream = tokio_stream::StreamExt::filter_map(stream, |res| res.ok());
    
    let mut transcription_stream = match transcriber.stream_transcription(Box::pin(bytes_stream)).await {
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
