use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fs;
use std::path::PathBuf;
use chrono::{DateTime, Local};
use directories::ProjectDirs;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct HistoryEntry {
    pub text: String,
    pub timestamp: DateTime<Local>,
}

pub struct HistoryManager {
    pub entries: VecDeque<HistoryEntry>,
    path: PathBuf,
}

impl HistoryManager {
    pub fn load() -> Self {
        let path = Self::get_history_path();
        let entries = if path.exists() {
            fs::read_to_string(&path)
                .ok()
                .and_then(|data| serde_json::from_str::<VecDeque<HistoryEntry>>(&data).ok())
                .unwrap_or_default()
        } else {
            VecDeque::new()
        };

        Self { entries, path }
    }

    fn get_history_path() -> PathBuf {
        let proj_dirs = ProjectDirs::from("org", "gnome", "dictation")
            .expect("Could not find project directories");
        let cache_dir = proj_dirs.cache_dir();
        fs::create_dir_all(cache_dir).ok();
        cache_dir.join("history.json")
    }

    pub fn add_entry(&mut self, text: String) {
        if text.trim().is_empty() {
            return;
        }

        let entry = HistoryEntry {
            text,
            timestamp: Local::now(),
        };

        self.entries.push_front(entry);
        if self.entries.len() > 10 {
            self.entries.pop_back();
        }

        self.save();
    }

    fn save(&self) {
        if let Ok(data) = serde_json::to_string_pretty(&self.entries) {
            let _ = fs::write(&self.path, data);
        }
    }

    pub fn menu_items(&self) -> Vec<(String, String)> {
        self.entries
            .iter()
            .enumerate()
            .map(|(i, entry)| {
                let time = entry.timestamp.format("%H:%M").to_string();
                let preview = if entry.text.chars().count() > 20 {
                    let mut s: String = entry.text.chars().take(20).collect();
                    s.push_str("...");
                    s
                } else {
                    entry.text.clone()
                };
                
                let label = format!("{} - \"{}\"", time, preview);
                (format!("history_{}", i), label)
            })
            .collect()
    }
}
