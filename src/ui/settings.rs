use gtk4::prelude::*;
use libadwaita::prelude::*;
use libadwaita::{Application, ApplicationWindow, PreferencesGroup, ActionRow, PreferencesPage, EntryRow, HeaderBar, ToolbarView, ComboRow, PasswordEntryRow};
use gtk4::{Box as GtkBox, Orientation, Button, FileDialog, glib, StringList};
use std::sync::{Arc, Mutex};
use tokio::runtime::Runtime;
use crate::config::{Config, BackendConfig, LlmConfig};
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

    // Transcription Backend
    let whisper_group = PreferencesGroup::builder()
        .title("Transcription Backend")
        .build();

    let backend_list = StringList::new(&["Local whisper.cpp", "OpenAI Whisper API"]);
    let backend_combo = ComboRow::builder()
        .title("Service")
        .model(&backend_list)
        .build();

    let (initial_backend_idx, initial_whisper_url, initial_openai_url, initial_openai_api_key, initial_openai_model) = {
        let cfg = config.lock().unwrap();
        match &cfg.backend {
            BackendConfig::WhisperCpp { url } => (0, url.clone(), "https://api.openai.com/v1/audio/transcriptions".to_string(), "".to_string(), "whisper-1".to_string()),
            BackendConfig::OpenAi { url, api_key, model } => (1, "http://localhost:58080".to_string(), url.clone(), api_key.clone(), model.clone()),
        }
    };

    backend_combo.set_selected(initial_backend_idx);

    let whisper_url_row = EntryRow::builder()
        .title("Whisper Server URL")
        .text(&initial_whisper_url)
        .visible(initial_backend_idx == 0)
        .build();

    let openai_url_row = EntryRow::builder()
        .title("OpenAI API URL")
        .text(&initial_openai_url)
        .visible(initial_backend_idx == 1)
        .build();

    let openai_api_key_row = PasswordEntryRow::builder()
        .title("OpenAI API Key")
        .text(&initial_openai_api_key)
        .visible(initial_backend_idx == 1)
        .build();

    let openai_model_row = EntryRow::builder()
        .title("OpenAI Model")
        .text(&initial_openai_model)
        .visible(initial_backend_idx == 1)
        .build();

    // 1. Backend Selection Callback
    let config_clone = config.clone();
    let whisper_url_row_clone = whisper_url_row.clone();
    let openai_url_row_clone = openai_url_row.clone();
    let openai_api_key_row_clone = openai_api_key_row.clone();
    let openai_model_row_clone = openai_model_row.clone();

    backend_combo.connect_selected_notify(move |combo| {
        let selected = combo.selected();
        whisper_url_row_clone.set_visible(selected == 0);
        openai_url_row_clone.set_visible(selected == 1);
        openai_api_key_row_clone.set_visible(selected == 1);
        openai_model_row_clone.set_visible(selected == 1);

        let mut cfg = config_clone.lock().unwrap();
        if selected == 0 {
            cfg.backend = BackendConfig::WhisperCpp {
                url: whisper_url_row_clone.text().to_string(),
            };
        } else {
            cfg.backend = BackendConfig::OpenAi {
                url: openai_url_row_clone.text().to_string(),
                api_key: openai_api_key_row_clone.text().to_string(),
                model: openai_model_row_clone.text().to_string(),
            };
        }
        cfg.save();
    });

    // 2. Field Update Callbacks
    let config_clone = config.clone();
    whisper_url_row.connect_text_notify(move |row| {
        let mut cfg = config_clone.lock().unwrap();
        if let BackendConfig::WhisperCpp { .. } = cfg.backend {
            cfg.backend = BackendConfig::WhisperCpp {
                url: row.text().to_string(),
            };
            cfg.save();
        }
    });

    let config_clone = config.clone();
    openai_url_row.connect_text_notify(move |row| {
        let mut cfg = config_clone.lock().unwrap();
        if let BackendConfig::OpenAi { api_key, model, .. } = &cfg.backend {
            cfg.backend = BackendConfig::OpenAi {
                url: row.text().to_string(),
                api_key: api_key.clone(),
                model: model.clone(),
            };
            cfg.save();
        }
    });

    let config_clone = config.clone();
    openai_api_key_row.connect_text_notify(move |row| {
        let mut cfg = config_clone.lock().unwrap();
        if let BackendConfig::OpenAi { url, model, .. } = &cfg.backend {
            cfg.backend = BackendConfig::OpenAi {
                url: url.clone(),
                api_key: row.text().to_string(),
                model: model.clone(),
            };
            cfg.save();
        }
    });

    let config_clone = config.clone();
    openai_model_row.connect_text_notify(move |row| {
        let mut cfg = config_clone.lock().unwrap();
        if let BackendConfig::OpenAi { url, api_key, .. } = &cfg.backend {
            cfg.backend = BackendConfig::OpenAi {
                url: url.clone(),
                api_key: api_key.clone(),
                model: row.text().to_string(),
            };
            cfg.save();
        }
    });

    whisper_group.add(&backend_combo);
    whisper_group.add(&whisper_url_row);
    whisper_group.add(&openai_url_row);
    whisper_group.add(&openai_api_key_row);
    whisper_group.add(&openai_model_row);

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
    let start_browse = Button::builder()
        .icon_name("document-open-symbolic")
        .valign(gtk4::Align::Center)
        .has_frame(false)
        .tooltip_text("Browse…")
        .build();
    let start_sound_row_clone = start_sound_row.clone();
    let window_clone = window.clone();
    start_browse.connect_clicked(move |_| {
        let row = start_sound_row_clone.clone();
        let dialog = FileDialog::builder().title("Select Start Sound").build();
        dialog.open(Some(&window_clone), gio::Cancellable::NONE, move |result| {
            if let Ok(file) = result {
                if let Some(path) = file.path() {
                    row.set_text(&path.to_string_lossy());
                }
            }
        });
    });
    start_sound_row.add_suffix(&start_browse);

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
    let end_browse = Button::builder()
        .icon_name("document-open-symbolic")
        .valign(gtk4::Align::Center)
        .has_frame(false)
        .tooltip_text("Browse…")
        .build();
    let end_sound_row_clone = end_sound_row.clone();
    let window_clone = window.clone();
    end_browse.connect_clicked(move |_| {
        let row = end_sound_row_clone.clone();
        let dialog = FileDialog::builder().title("Select End Sound").build();
        dialog.open(Some(&window_clone), gio::Cancellable::NONE, move |result| {
            if let Ok(file) = result {
                if let Some(path) = file.path() {
                    row.set_text(&path.to_string_lossy());
                }
            }
        });
    });
    end_sound_row.add_suffix(&end_browse);

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
    // TODO: add LLM provider (Ollama) settings UI once the feature is implemented
    // page.add(&llm_group);
    page.add(&sound_group);

    content_vbox.append(&page);

    toolbar_view.set_content(Some(&content_vbox));
    window.set_content(Some(&toolbar_view));
    window.present();
}
