use clap::{Parser, Subcommand};
use libadwaita::prelude::*;
use libadwaita::Application;
use gtk4::glib;
use std::sync::Arc;
use tokio::runtime::Runtime;
use std::path::PathBuf;

mod recorder;
mod accessibility;
mod config;
mod extension_proxy;
mod audio;
mod history;
mod mpris;
mod traits;
mod transcriber_whisper;
mod transcriber_factory;
mod vad;
mod ui;
mod daemon;
mod commands;

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
    #[command(name = "at-spi")]
    AtSpi,
    /// Watch AT-SPI focus events in real time and print cursor context as focus changes
    #[command(name = "at-spi-watcher")]
    AtSpiWatcher,
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

use commands::extension::ExtensionCommands;
use commands::mpris::MprisCommands;

fn main() -> glib::ExitCode {
    let args = Cli::parse();

    match args.command {
        Some(Commands::Microphone { duration }) => {
            let rt = Runtime::new().expect("Failed to create Tokio runtime");
            rt.block_on(commands::microphone::run(duration));
            glib::ExitCode::SUCCESS
        }
        Some(Commands::AtSpi) => {
            let rt = Runtime::new().expect("Failed to create Tokio runtime");
            rt.block_on(commands::at_spi::snapshot());
            glib::ExitCode::SUCCESS
        }
        Some(Commands::AtSpiWatcher) => {
            let rt = Runtime::new().expect("Failed to create Tokio runtime");
            rt.block_on(commands::at_spi::watcher());
            glib::ExitCode::SUCCESS
        }
        Some(Commands::Transcribe { path }) => {
            let rt = Runtime::new().expect("Failed to create Tokio runtime");
            rt.block_on(commands::transcribe::run(path));
            glib::ExitCode::SUCCESS
        }
        Some(Commands::Extension { subcommand }) => {
            let rt = Runtime::new().expect("Failed to create Tokio runtime");
            rt.block_on(commands::extension::run(subcommand));
            glib::ExitCode::SUCCESS
        }
        Some(Commands::Sound { path }) => {
            commands::audio::play_sound(path);
            glib::ExitCode::SUCCESS
        }
        Some(Commands::Volume { level }) => {
            commands::audio::volume(level);
            glib::ExitCode::SUCCESS
        }
        Some(Commands::Mpris { subcommand }) => {
            let rt = Runtime::new().expect("Failed to create Tokio runtime");
            rt.block_on(commands::mpris::run(subcommand));
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

