use gtk4::prelude::*;
use libadwaita::prelude::*;
use libadwaita::{Application, ApplicationWindow, PreferencesGroup, ActionRow, PreferencesPage, EntryRow, HeaderBar, ToolbarView};
use gtk4::{Box as GtkBox, Orientation, Button, glib};
use std::sync::{Arc, Mutex};
use tokio::runtime::Runtime;
use crate::config::{Config, BackendConfig, LlmConfig};
use crate::recorder;
use crate::transcriber_whisper::WhisperClient;
use crate::traits::Transcriber;
use crate::ui::shortcut_recorder;

pub fn build_ui(app: &Application, runtime: Arc<Runtime>) {
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
            let conn = zbus::Connection::session().await.ok();
            if let Some(c) = conn {
                if let Ok(proxy) = crate::extension_proxy::ExtensionProxy::new(&c).await {
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
        let shortcut_row_inner = shortcut_row_clone.clone();
        shortcut_recorder::record_shortcut(&window_clone, config_clone.clone(), runtime_clone.clone(), move |accel| {
            shortcut_row_inner.set_subtitle(&glib::markup_escape_text(&accel));
        });
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

    let initial_recording_color = {
        let cfg = config.lock().unwrap();
        cfg.recording_color.clone()
    };

    let recording_color_row = EntryRow::builder()
        .title("Recording Color (Hex)")
        .text(&initial_recording_color)
        .build();

    let config_clone = config.clone();
    recording_color_row.connect_text_notify(move |row| {
        let mut cfg = config_clone.lock().unwrap();
        cfg.recording_color = row.text().to_string();
        cfg.save();
    });
    general_group.add(&recording_color_row);

    let initial_transcribing_color = {
        let cfg = config.lock().unwrap();
        cfg.transcribing_color.clone()
    };

    let transcribing_color_row = EntryRow::builder()
        .title("Transcribing Color (Hex)")
        .text(&initial_transcribing_color)
        .build();

    let config_clone = config.clone();
    transcribing_color_row.connect_text_notify(move |row| {
        let mut cfg = config_clone.lock().unwrap();
        cfg.transcribing_color = row.text().to_string();
        cfg.save();
    });
    general_group.add(&transcribing_color_row);

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
            
            let client = WhisperClient::new(url);
            
            std::mem::forget(stream);

            let mut transcription_stream = match <WhisperClient as Transcriber>::stream_transcription(&client, Box::pin(audio_stream)).await {
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
