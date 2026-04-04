use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use reqwest::Client;
use serde::Deserialize;
use std::pin::Pin;
use async_trait::async_trait;
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
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut writer = hound::WavWriter::new(&mut cursor, spec)
                .expect("Failed to create WAV writer");
            let samples: &[f32] = bytemuck::cast_slice(raw_samples);
            for &sample in samples {
                writer.write_sample(sample).expect("Failed to write sample");
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
        let mut audio_stream = audio_stream;
        let mut raw_samples = Vec::new();
        while let Some(chunk) = audio_stream.next().await {
            raw_samples.extend_from_slice(&chunk);
        }

        let wav_bytes = Self::build_wav(format, &raw_samples);

        // Save last recording to RAM for debugging (e.g. aplay /dev/shm/dictation_last.wav)
        if let Err(e) = std::fs::write("/dev/shm/dictation_last.wav", &wav_bytes) {
            eprintln!("[debug] Failed to save debug WAV: {e}");
        }

        let mut target_url = self.url.clone();
        if !target_url.contains("/inference") {
            if !target_url.ends_with('/') {
                target_url.push('/');
            }
            target_url.push_str("inference");
        }

        let mut query_params = Vec::new();
        if let Some(prompt) = &self.prompt {
            query_params.push(("prompt", prompt.as_str()));
        }

        let response = self.client
            .post(&target_url)
            .query(&query_params)
            .header("Content-Type", "audio/wav")
            .body(wav_bytes)
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
