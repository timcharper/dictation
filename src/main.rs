use clap::{Parser, Subcommand};
use libadwaita::prelude::*;
use libadwaita::{Application, ApplicationWindow, PreferencesGroup, ActionRow, PreferencesPage};
use gtk4::{glib, Box as GtkBox, Orientation, Button};
use std::sync::Arc;
use tokio::runtime::Runtime;
use tokio_stream::StreamExt;
use std::env;

mod recorder;
mod transcriber;

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
        None => {
            // Run the GTK app
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
        if let Some(bytes) = output.audio_stream.next().await {
            let chunk: &[f32] = bytemuck::cast_slice(&bytes);
            samples.extend_from_slice(chunk);
        }
    }

    // Stop recording
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

fn build_ui(app: &Application, runtime: Arc<Runtime>) {
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

    let whisper_group = PreferencesGroup::builder()
        .title("Transcription (Whisper)")
        .build();
    
    let whisper_url_row = ActionRow::builder()
        .title("Whisper Server URL")
        .subtitle("http://localhost:8080")
        .build();
    whisper_group.add(&whisper_url_row);

    let llm_group = PreferencesGroup::builder()
        .title("LLM Provider (Ollama)")
        .build();
    
    let ollama_url_row = ActionRow::builder()
        .title("Ollama Server URL")
        .subtitle("http://localhost:11434")
        .build();
    llm_group.add(&ollama_url_row);

    page.add(&general_group);
    page.add(&whisper_group);
    page.add(&llm_group);

    vbox.append(&page);

    let start_button = Button::builder()
        .label("Start Recording (Test Stream)")
        .css_classes(vec!["suggested-action"])
        .build();

    let rt_clone = runtime.clone();
    start_button.connect_clicked(move |_| {
        let rt = rt_clone.clone();
        rt.spawn(async move {
            let recorder = recorder::AudioRecorder::new();
            let output = recorder.start_recording();
            let client = transcriber::WhisperClient::new("http://localhost:8080".to_string());
            
            let mut transcription_stream = match client.stream_transcription(Box::pin(output.audio_stream)).await {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Failed to start transcription stream: {:?}", e);
                    return;
                }
            };

            while let Some(res) = transcription_stream.next().await {
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
