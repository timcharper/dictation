use directories::BaseDirs;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Config {
    #[serde(default)]
    pub backend: BackendConfig,
    #[serde(default)]
    pub llm: LlmConfig,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BackendConfig {
    WhisperCpp { url: String },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LlmConfig {
    Ollama { url: String, model: String },
}

impl Default for BackendConfig {
    fn default() -> Self {
        Self::WhisperCpp {
            url: "http://localhost:8080".to_string(),
        }
    }
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self::Ollama {
            url: "http://localhost:11434".to_string(),
            model: "llama3".to_string(),
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            backend: BackendConfig::default(),
            llm: LlmConfig::default(),
        }
    }
}

impl Config {
    pub fn path() -> PathBuf {
        let base_dirs = BaseDirs::new().expect("Could not determine home directory");
        let mut path = base_dirs.config_dir().to_path_buf();
        path.push("dictation");
        path.push("dictation.toml");
        path
    }

    pub fn load() -> Self {
        let path = Self::path();
        if path.exists() {
            let content = fs::read_to_string(&path).expect("Failed to read config file");
            match toml::from_str::<Config>(&content) {
                Ok(config) => config,
                Err(e) => {
                    eprintln!(
                        "Warning: Failed to parse config file: {}. Using defaults.",
                        e
                    );
                    Self::default()
                }
            }
        } else {
            let config = Self::default();
            config.save();
            config
        }
    }

    pub fn save(&self) {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("Failed to create config directory");
        }
        let content = toml::to_string_pretty(self).expect("Failed to serialize config");
        fs::write(path, content).expect("Failed to write config file");
    }
}
