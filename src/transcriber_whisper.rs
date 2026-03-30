use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use reqwest::Client;
use serde::Deserialize;
use std::pin::Pin;
use async_trait::async_trait;
use crate::traits::{Transcriber, TranscriptionResult};

#[derive(Debug, Deserialize)]
struct InternalTranscriptionResponse {
    pub text: String,
    #[serde(default)]
    pub is_final: bool,
}

impl InternalTranscriptionResponse {
    pub fn sanitize(&mut self) {
        let text = self.text.chars().map(|c| match c {
            '\r' | '\n' | '\x0B' | '\x0C' | '\u{0085}' | '\u{2028}' | '\u{2029}' => ' ',
            _ => c,
        }).collect::<String>();
        
        self.text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    }
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
}

#[async_trait]
impl Transcriber for WhisperClient {
    async fn stream_transcription(
        &self,
        audio_stream: Pin<Box<dyn Stream<Item = Bytes> + Send>>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<TranscriptionResult, String>> + Send>>, String> {
        let try_stream = audio_stream.map(|b| Ok::<Bytes, std::io::Error>(b));
        
        let mut target_url = self.url.clone();
        if !target_url.contains("/inference") {
            if !target_url.ends_with('/') {
                target_url.push('/');
            }
            target_url.push_str("inference");
        }

        let response = self.client
            .post(&target_url)
            .header("Content-Type", "audio/wav")
            .body(reqwest::Body::wrap_stream(try_stream))
            .send()
            .await
            .map_err(|e| e.to_string())?;

        let stream = response.bytes_stream().map(|res| {
            res.map_err(|e| e.to_string())
                .and_then(|b| {
                    let mut resp = serde_json::from_slice::<InternalTranscriptionResponse>(&b)
                        .map_err(|e| {
                            format!("JSON parse error: {} (body: {})", e, String::from_utf8_lossy(&b))
                        })?;
                    resp.sanitize();
                    Ok(TranscriptionResult {
                        text: resp.text,
                        is_final: resp.is_final,
                    })
                })
        });

        Ok(Box::pin(stream))
    }
}
