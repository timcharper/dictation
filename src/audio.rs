use rodio::{Decoder, OutputStream, Sink};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use pulsectl::controllers::SinkController;
use pulsectl::controllers::DeviceControl;
use crate::config::SoundConfig;

pub struct AudioManager {
}

impl AudioManager {
    pub fn new() -> Self {
        Self {
        }
    }

    pub fn get_volume(&self) -> Option<f64> {
        let mut controller = SinkController::create().ok()?;
        let default_sink = controller.get_default_device().ok()?;
        let vol = default_sink.volume.avg().0;
        let max = libpulse_binding::volume::Volume::NORMAL.0 as f64;
        Some(vol as f64 / max)
    }

    pub fn set_volume(&self, volume: f64) {
        if let Ok(mut controller) = SinkController::create() {
            if let Ok(default_sink) = controller.get_default_device() {
                let max = libpulse_binding::volume::Volume::NORMAL.0 as f64;
                let target_vol = (volume * max) as u32;
                let mut vol = default_sink.volume.clone();
                for channel in vol.get_mut() {
                    channel.0 = target_vol;
                }
                let _ = controller.set_device_volume_by_index(default_sink.index, &vol);
            }
        }
    }

    pub fn play_sound(&self, path: impl AsRef<Path>) {
        let path_buf = path.as_ref().to_path_buf();
        std::thread::spawn(move || {
            let (_stream, handle) = match OutputStream::try_default() {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Failed to open default output stream: {:?}", e);
                    return;
                }
            };

            let file = match File::open(&path_buf) {
                Ok(f) => f,
                Err(e) => {
                    eprintln!("Failed to open sound file: {:?}", e);
                    return;
                }
            };

            let source = match Decoder::new(BufReader::new(file)) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Failed to decode sound file: {:?}", e);
                    return;
                }
            };

            if let Ok(sink) = Sink::try_new(&handle) {
                sink.append(source);
                sink.sleep_until_end();
            }
        });
    }

    pub fn play_sound_sync(&self, path: impl AsRef<Path>) {
        let path = path.as_ref();
        let (_stream, handle) = match OutputStream::try_default() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Failed to open default output stream: {:?}", e);
                return;
            }
        };

        let file = match File::open(path) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("Failed to open sound file: {:?}", e);
                return;
            }
        };

        let source = match Decoder::new(BufReader::new(file)) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Failed to decode sound file: {:?}", e);
                return;
            }
        };

        if let Ok(sink) = Sink::try_new(&handle) {
            sink.append(source);
            sink.sleep_until_end();
            // Add a small delay to ensure the hardware buffer is drained
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }

    pub async fn duck_and_play_start(&self, _proxy: &crate::extension_proxy::ExtensionProxy<'_>, config: &SoundConfig) -> Option<f64> {
        let original_volume = self.get_volume();

        // Play start sound at original volume
        if let Some(path) = &config.start_sound {
            self.play_sound_sync(path);
        }

        // Duck volume AFTER the sound has finished
        if original_volume.is_some() {
            self.set_volume(config.ducking_volume as f64);
        }

        original_volume
    }

    pub async fn restore_and_play_end(&self, _proxy: &crate::extension_proxy::ExtensionProxy<'_>, config: &SoundConfig, original_volume: Option<f64>) {
        if let Some(path) = &config.end_sound {
            self.play_sound(path);
        }

        if let Some(vol) = original_volume {
            self.set_volume(vol);
        }
    }
}
