use async_trait::async_trait;
use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use reqwest::multipart;
use serde::Deserialize;
use std::pin::Pin;
use tokio_stream::wrappers::ReceiverStream;

use crate::opus_encoder::OpusOggEncoder;
use crate::traits::{AudioFormat, Transcriber, TranscriptionResult};

#[derive(Debug, Deserialize)]
struct OpenAiResponse {
    pub text: String,
}

pub struct OpenAiClient {
    client: reqwest::Client,
    url: String,
    api_key: String,
    model: String,
    prompt: Option<String>,
}

impl OpenAiClient {
    pub fn new(url: String, api_key: String, model: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            url,
            api_key,
            model,
            prompt: None,
        }
    }
}

#[async_trait]
impl Transcriber for OpenAiClient {
    async fn stream_transcription(
        &self,
        _format: AudioFormat,
        audio_stream: Pin<Box<dyn Stream<Item = Bytes> + Send>>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<TranscriptionResult, String>> + Send>>, String> {
        let url = self.url.clone();
        let api_key = self.api_key.clone();
        let model = self.model.clone();
        let prompt = self.prompt.clone();
        let client = self.client.clone();

        let (tx, rx) = tokio::sync::mpsc::channel(10);

        tokio::spawn(async move {
            let encoder = OpusOggEncoder::new();
            let mut opus_stream = encoder.encode_stream(audio_stream);

            // We wrap the stream to convert Result<Bytes, String> to Result<Bytes, reqwest::Error>
            // or just use it directly if possible. reqwest::Body::wrap_stream wants Result<Bytes, Error>.
            let body_stream = opus_stream.map(|res| {
                res.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
            });

            let mut form = multipart::Form::new()
                .text("model", model)
                .part("file", multipart::Part::stream(reqwest::Body::wrap_stream(body_stream))
                    .file_name("audio.ogg")
                    .mime_str("audio/ogg")
                    .expect("Failed to set mime type"));

            if let Some(p) = prompt {
                form = form.text("prompt", p);
            }

            let response_res = client
                .post(&url)
                .bearer_auth(api_key)
                .multipart(form)
                .send()
                .await;

            match response_res {
                Ok(response) => {
                    if response.status().is_success() {
                        match response.json::<OpenAiResponse>().await {
                            Ok(res) => {
                                let _ = tx.send(Ok(TranscriptionResult {
                                    text: res.text,
                                    is_final: true,
                                })).await;
                            }
                            Err(e) => {
                                let _ = tx.send(Err(format!("OpenAI JSON error: {}", e))).await;
                            }
                        }
                    } else {
                        let status = response.status();
                        let error_text = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
                        let _ = tx.send(Err(format!("OpenAI API error ({}): {}", status, error_text))).await;
                    }
                }
                Err(e) => {
                    let _ = tx.send(Err(format!("OpenAI request error: {}", e))).await;
                }
            }
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    fn set_prompt(&mut self, prompt: String) {
        if !prompt.is_empty() {
            // Keep last 200 words
            let words: Vec<&str> = prompt.split_whitespace().collect();
            let truncated = if words.len() > 200 {
                words[words.len() - 200..].join(" ")
            } else {
                words.join(" ")
            };
            self.prompt = Some(truncated);
        }
    }
}
