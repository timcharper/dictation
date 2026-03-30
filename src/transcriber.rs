use bytes::Bytes;
use futures_util::Stream;
use reqwest::Client;
use serde::Deserialize;
use std::pin::Pin;
use tokio_stream::StreamExt;

#[derive(Debug, Deserialize)]
pub struct TranscriptionResponse {
    pub text: String,
    #[serde(default)]
    pub is_final: bool,
}

impl TranscriptionResponse {
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

    pub async fn stream_transcription(
        &self,
        audio_stream: Pin<Box<dyn Stream<Item = Bytes> + Send>>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<TranscriptionResponse, String>> + Send>>, reqwest::Error> {
        let try_stream = audio_stream.map(|b| Ok::<Bytes, std::io::Error>(b));
        
        // Ensure the URL has the /inference path if it's whisper.cpp server
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
            .await?;

        // whisper.cpp server may return multiple JSON objects on separate lines
        // For now, we process them as they come in the byte stream
        let stream = response.bytes_stream().map(|res| {
            res.map_err(|e| e.to_string())
                .and_then(|b| {
                    // Try to deserialize the whole chunk. 
                    // whisper.cpp /inference usually returns a single JSON at the end if not streaming.
                    // But if it returns multiple, we might need a more sophisticated line-by-line parser.
                    let mut resp = serde_json::from_slice::<TranscriptionResponse>(&b)
                        .map_err(|e| {
                            // Fallback: maybe it's just raw text or multiple JSONs?
                            // For now, let's just report the error if it's not valid JSON
                            format!("JSON parse error: {} (body: {})", e, String::from_utf8_lossy(&b))
                        })?;
                    resp.sanitize();
                    Ok(resp)
                })
        });

        Ok(Box::pin(stream))
    }
}
