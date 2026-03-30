use crate::config::BackendConfig;
use crate::traits::Transcriber;
use crate::transcriber_whisper::WhisperClient;
use std::sync::Arc;

pub fn create_transcriber(config: &BackendConfig) -> Arc<dyn Transcriber> {
    match config {
        BackendConfig::WhisperCpp { url } => Arc::new(WhisperClient::new(url.clone())),
    }
}
