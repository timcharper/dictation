use std::path::PathBuf;
use crate::audio::AudioManager;

pub fn play_sound(path: PathBuf) {
    let audio_mgr = AudioManager::new();
    audio_mgr.play_sound(path);
    std::thread::sleep(std::time::Duration::from_secs(3));
}

pub fn volume(level: Option<f64>) {
    let audio_mgr = AudioManager::new();
    if let Some(l) = level {
        audio_mgr.set_volume(l);
        println!("Volume set to {}", l);
    } else if let Some(v) = audio_mgr.get_volume() {
        println!("Current volume: {:.2}", v);
    } else {
        println!("Failed to get volume");
    }
}
