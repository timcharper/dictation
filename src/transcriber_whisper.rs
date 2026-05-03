use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use reqwest::Client;
use serde::Deserialize;
use std::pin::Pin;
use async_trait::async_trait;
use tokio_stream::wrappers::ReceiverStream;
use crate::traits::{AudioFormat, Transcriber, TranscriptionResult};

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
    prompt: Option<String>,
}

impl WhisperClient {
    pub fn new(url: String) -> Self {
        Self {
            client: Client::new(),
            url,
            prompt: None,
        }
    }

    fn build_wav(format: AudioFormat, raw_samples: &[u8]) -> Vec<u8> {
        let spec = hound::WavSpec {
            channels: format.channels,
            sample_rate: format.sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut writer = hound::WavWriter::new(&mut cursor, spec)
                .expect("Failed to create WAV writer");
            let samples: &[f32] = bytemuck::cast_slice(raw_samples);
            for &sample in samples {
                let sample_i16 = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
                writer.write_sample(sample_i16).expect("Failed to write sample");
            }
            writer.finalize().expect("Failed to finalize WAV");
        }
        cursor.into_inner()
    }
}

#[async_trait]
impl Transcriber for WhisperClient {
    async fn stream_transcription(
        &self,
        format: AudioFormat,
        audio_stream: Pin<Box<dyn Stream<Item = Bytes> + Send>>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<TranscriptionResult, String>> + Send>>, String> {
        let mut target_url = self.url.clone();
        if !target_url.contains("/inference") {
            if !target_url.ends_with('/') {
                target_url.push('/');
            }
            target_url.push_str("inference");
        }

        let mut query_params = Vec::new();
        if let Some(prompt) = &self.prompt {
            query_params.push(("prompt", prompt.clone()));
        }

        let client = self.client.clone();
        let (tx, rx) = tokio::sync::mpsc::channel(100);

        tokio::spawn(async move {
            let mut audio_stream = audio_stream;
            let mut raw_samples = Vec::new();
            while let Some(chunk) = audio_stream.next().await {
                raw_samples.extend_from_slice(&chunk);
            }

            let wav_bytes = Self::build_wav(format, &raw_samples);

            // Save last recording to RAM for debugging
            if let Err(e) = std::fs::write("/dev/shm/dictation_last.wav", &wav_bytes) {
                eprintln!("[debug] Failed to save debug WAV: {e}");
            }

            let response_res = client
                .post(&target_url)
                .query(&query_params)
                .header("Content-Type", "audio/wav")
                .body(wav_bytes)
                .send()
                .await;

            match response_res {
                Ok(response) => {
                    let mut bytes_stream = parse_whisper_stream(response.bytes_stream());
                    while let Some(res) = bytes_stream.next().await {
                        if tx.send(res).await.is_err() {
                            break;
                        }
                    }
                }
                Err(e) => {
                    let _ = tx.send(Err(e.to_string())).await;
                }
            }
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    fn set_prompt(&mut self, prompt: String) {
        if !prompt.is_empty() {
            // Sanitization: replace em dash with hyphen, keep alphanumeric and punctuation
            let sanitized: String = prompt
                .chars()
                .map(|c| match c {
                    '—' => '-',
                    c if c.is_alphanumeric() || c.is_ascii_punctuation() || c == ' ' => c,
                    _ => ' ',
                })
                .collect();

            // Keep last 200 words before the cursor
            let words: Vec<&str> = sanitized.split_whitespace().collect();
            let truncated = if words.len() > 200 {
                words[words.len() - 200..].join(" ")
            } else {
                words.join(" ")
            };

            if !truncated.is_empty() {
                println!("[INFO] Whisper prompt context (sanitized): '{}'", truncated);
                self.prompt = Some(truncated);
            }
        }
    }
}

fn parse_whisper_stream<S, E>(
    bytes_stream: S,
) -> Pin<Box<dyn Stream<Item = Result<TranscriptionResult, String>> + Send>>
where
    S: Stream<Item = Result<Bytes, E>> + Send + Unpin + 'static,
    E: std::fmt::Display + 'static,
{
    let buffer = Vec::new();

    let stream = futures_util::stream::unfold(
        (bytes_stream, buffer),
        |(mut bytes_stream, mut buffer)| async move {
            loop {
                if !buffer.is_empty() {
                    let mut de = serde_json::Deserializer::from_slice(&buffer).into_iter::<InternalTranscriptionResponse>();
                    if let Some(res) = de.next() {
                        match res {
                            Ok(mut resp) => {
                                let consumed = de.byte_offset();
                                buffer.drain(..consumed);
                                resp.sanitize();
                                let result = Ok(TranscriptionResult {
                                    text: resp.text,
                                    is_final: resp.is_final,
                                });
                                return Some((result, (bytes_stream, buffer)));
                            }
                            Err(e) if e.is_eof() => {
                                // Need more data
                            }
                            Err(e) => {
                                let body_snippet = String::from_utf8_lossy(&buffer);
                                let body_snippet = if body_snippet.len() > 100 {
                                    format!("{}...", &body_snippet[..100])
                                } else {
                                    body_snippet.to_string()
                                };
                                let result = Err(format!("JSON parse error: {} (body: {})", e, body_snippet));
                                buffer.clear();
                                return Some((result, (bytes_stream, buffer)));
                            }
                        }
                    }
                }

                match bytes_stream.next().await {
                    Some(Ok(b)) => buffer.extend_from_slice(&b),
                    Some(Err(e)) => return Some((Err(e.to_string()), (bytes_stream, buffer))),
                    None => return None,
                }
            }
        },
    );

    Box::pin(stream)
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::stream;

    #[tokio::test]
    async fn test_parse_whisper_stream_chunked() {
        let json_part1 = r#"{"text": "Hello "#;
        let json_part2 = r#"world!", "is_final": true}"#;
        
        let chunks: Vec<Result<Bytes, String>> = vec![
            Ok(Bytes::from(json_part1)),
            Ok(Bytes::from(json_part2)),
        ];
        let bytes_stream = stream::iter(chunks);
        
        let mut transcription_stream = parse_whisper_stream(bytes_stream);
        
        let res = transcription_stream.next().await.unwrap().unwrap();
        assert_eq!(res.text, "Hello world!");
        assert!(res.is_final);
        assert!(transcription_stream.next().await.is_none());
    }

    #[tokio::test]
    async fn test_parse_whisper_stream_multiple_messages() {
        let msg1 = r#"{"text": "Partial", "is_final": false}"#;
        let msg2 = r#"{"text": "Final result", "is_final": true}"#;
        
        // Split msg1 across chunks, msg2 in one chunk
        let chunks: Vec<Result<Bytes, String>> = vec![
            Ok(Bytes::from(&msg1[..10])),
            Ok(Bytes::from(&msg1[10..])),
            Ok(Bytes::from(msg2)),
        ];
        let bytes_stream = stream::iter(chunks);
        
        let mut transcription_stream = parse_whisper_stream(bytes_stream);
        
        let res1 = transcription_stream.next().await.unwrap().unwrap();
        assert_eq!(res1.text, "Partial");
        assert!(!res1.is_final);
        
        let res2 = transcription_stream.next().await.unwrap().unwrap();
        assert_eq!(res2.text, "Final result");
        assert!(res2.is_final);
        
        assert!(transcription_stream.next().await.is_none());
    }
}
