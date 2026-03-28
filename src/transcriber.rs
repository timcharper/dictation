use bytes::Bytes;
use futures_util::Stream;
use reqwest::Client;
use serde::Deserialize;
use std::pin::Pin;
use tokio_stream::StreamExt;

#[derive(Debug, Deserialize)]
pub struct TranscriptionResponse {
    pub text: String,
    pub is_final: bool,
}

pub struct WhisperClient {
    client: Client,
    url: String,
}

impl WhisperClient {
    pub fn new(url: String) -> Self {
        Self {
            client: Client::new(),
            url,
        }
    }

    pub async fn stream_transcription(
        &self,
        audio_stream: Pin<Box<dyn Stream<Item = Bytes> + Send>>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<TranscriptionResponse, String>> + Send>>, reqwest::Error> {
        let try_stream = audio_stream.map(|b| Ok::<Bytes, std::io::Error>(b));
        
        let response = self.client
            .post(&self.url)
            .body(reqwest::Body::wrap_stream(try_stream))
            .send()
            .await?;

        let stream = response.bytes_stream().map(|res| {
            res.map_err(|e| e.to_string())
                .and_then(|b| {
                    serde_json::from_slice::<TranscriptionResponse>(&b)
                        .map_err(|e| e.to_string())
                })
        });

        Ok(Box::pin(stream))
    }
}
