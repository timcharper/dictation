use std::path::PathBuf;
use bytes::Bytes;
use crate::traits::AudioFormat;

pub async fn run(path: PathBuf) {
    println!("Transcribing file: {:?}", path);

    let config = crate::config::Config::load();
    let transcriber = crate::transcriber_factory::create_transcriber(&config.backend, None);

    let mut reader = hound::WavReader::open(&path).expect("Failed to open WAV file");
    let spec = reader.spec();
    let format = AudioFormat { sample_rate: spec.sample_rate, channels: spec.channels };

    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().map(|s| s.expect("read sample")).collect(),
        hound::SampleFormat::Int => reader.samples::<i32>().map(|s| {
            let s = s.expect("read sample");
            s as f32 / i32::MAX as f32
        }).collect(),
    };

    let raw_bytes = Bytes::from(bytemuck::cast_slice::<f32, u8>(&samples).to_vec());
    let audio_stream = futures_util::stream::once(async move { raw_bytes });

    let mut transcription_stream = match transcriber.stream_transcription(format, Box::pin(audio_stream)).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to start transcription stream: {:?}", e);
            return;
        }
    };

    while let Some(res) = futures_util::StreamExt::next(&mut transcription_stream).await {
        match res {
            Ok(resp) => {
                if !resp.text.is_empty() {
                    println!("Transcription: {}", resp.text);
                }
            }
            Err(e) => eprintln!("Transcription error: {:?}", e),
        }
    }
}
