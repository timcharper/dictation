use std::env;
use bytemuck;
use hound;

pub async fn run(duration_secs: u64) {
    println!("Recording for {} seconds...", duration_secs);

    let recorder = crate::recorder::AudioRecorder::new();
    let mut output = recorder.start_recording();

    let mut samples: Vec<f32> = Vec::new();
    let start_time = std::time::Instant::now();

    while start_time.elapsed().as_secs() < duration_secs {
        if let Some(bytes) = tokio_stream::StreamExt::next(&mut output.audio_stream).await {
            let chunk: &[f32] = bytemuck::cast_slice(&bytes);
            samples.extend_from_slice(chunk);
        }
    }

    drop(output.stream);

    let temp_dir = env::temp_dir();
    let file_path = temp_dir.join("dictation_test.wav");

    let spec = hound::WavSpec {
        channels: output.config.channels as u16,
        sample_rate: output.config.sample_rate.0,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };

    let mut writer = hound::WavWriter::create(&file_path, spec).expect("Failed to create WAV writer");
    for sample in samples {
        writer.write_sample(sample).expect("Failed to write sample");
    }
    writer.finalize().expect("Failed to finalize WAV file");

    println!("Success! Recorded to: {:?}", file_path);
}
