use rodio::{Decoder, OutputStream, Sink};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use crate::extension_proxy::ExtensionProxy;
use crate::config::SoundConfig;

pub struct AudioManager {
    _stream: OutputStream,
    _handle: rodio::OutputStreamHandle,
}

impl AudioManager {
    pub fn new() -> Self {
        let (stream, handle) = OutputStream::try_default().expect("Failed to open default output stream");
        Self {
            _stream: stream,
            _handle: handle,
        }
    }

    pub fn play_sound(&self, path: impl AsRef<Path>) {
        let path = path.as_ref();
        if !path.exists() {
            eprintln!("Sound file not found: {:?}", path);
            return;
        }

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

        let sink = Sink::try_new(&self._handle).expect("Failed to create sink");
        sink.append(source);
        sink.detach();
    }

    pub async fn duck_and_play_start(&self, proxy: &ExtensionProxy<'_>, config: &SoundConfig) -> Option<f64> {
        let original_volume = match proxy.get_volume().await {
            Ok(v) => Some(v),
            Err(e) => {
                eprintln!("Failed to get original volume: {:?}", e);
                None
            }
        };

        if original_volume.is_some() {
            let _ = proxy.set_volume(config.ducking_volume as f64).await;
        }

        if let Some(path) = &config.start_sound {
            self.play_sound(path);
        }

        original_volume
    }

    pub async fn restore_and_play_end(&self, proxy: &ExtensionProxy<'_>, config: &SoundConfig, original_volume: Option<f64>) {
        if let Some(path) = &config.end_sound {
            self.play_sound(path);
        }

        if let Some(vol) = original_volume {
            let _ = proxy.set_volume(vol).await;
        }
    }
}
