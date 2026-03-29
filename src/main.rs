use clap::{Parser, Subcommand};
use libadwaita::prelude::*;
use libadwaita::{Application, ApplicationWindow, PreferencesGroup, ActionRow, PreferencesPage, EntryRow};
use gtk4::{glib, Box as GtkBox, Orientation, Button};
use std::sync::{Arc, Mutex};
use tokio::runtime::Runtime;
// use tokio_stream::StreamExt;
use std::env;

mod recorder;
mod transcriber;
mod accessibility;
mod config;

use config::{Config, BackendConfig, LlmConfig};

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
        sample_rate: output.config.sample_rate,
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
        .default_height(400)
        .build();

    let vbox = GtkBox::builder()
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
    whisper_url_row.connect_apply(move |row| {
        let mut cfg = config_clone.lock().unwrap();
        cfg.backend = BackendConfig::WhisperCpp {
            url: row.text().to_string(),
        };
        cfg.save();
        println!("Whisper URL saved: {}", row.text());
    });
    // Also save on focus out or text change? 
    // connect_apply is usually enough if user presses Enter. 
    // To be safer, let's use notify::text
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

    page.add(&general_group);
    page.add(&whisper_group);
    page.add(&llm_group);

    vbox.append(&page);

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
            let output = recorder.start_recording();
            
            let url = {
                let cfg = config.lock().unwrap();
                match &cfg.backend {
                    BackendConfig::WhisperCpp { url } => url.clone(),
                }
            };
            
            let client = transcriber::WhisperClient::new(url);
            
            // use futures_util::StreamExt;
            let mut transcription_stream = match client.stream_transcription(Box::pin(output.audio_stream)).await {
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

    vbox.append(&start_button);
    window.set_content(Some(&vbox));
    window.present();
}
