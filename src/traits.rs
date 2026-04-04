use async_trait::async_trait;
use bytes::Bytes;
use futures_util::Stream;
use std::pin::Pin;

#[async_trait]
pub trait Transcriber: Send + Sync {
    async fn stream_transcription(
        &self,
        audio_stream: Pin<Box<dyn Stream<Item = Bytes> + Send>>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<TranscriptionResult, String>> + Send>>, String>;

    fn set_prompt(&mut self, _prompt: String) {}
}

#[derive(Debug)]
pub struct TranscriptionResult {
    pub text: String,
    pub is_final: bool,
}
