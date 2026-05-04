use crate::config::BackendConfig;
use crate::traits::Transcriber;
use crate::transcriber_whisper::WhisperClient;
use crate::transcriber_openai::OpenAiClient;
use std::sync::Arc;

pub fn create_transcriber(config: &BackendConfig, prompt: Option<String>) -> Arc<dyn Transcriber> {
    match config {
        BackendConfig::WhisperCpp { url } => {
            let mut client = WhisperClient::new(url.clone());
            if let Some(p) = prompt {
                client.set_prompt(p);
            }
            Arc::new(client)
        }
        BackendConfig::OpenAi { url, api_key, model } => {
            let mut client = OpenAiClient::new(url.clone(), api_key.clone(), model.clone());
            if let Some(p) = prompt {
                client.set_prompt(p);
            }
            Arc::new(client)
        }
    }
}
