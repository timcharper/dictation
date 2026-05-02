use earshot::Detector;
use rubato::{Resampler, FastFixedIn};

pub struct VadProcessor {
    detector: Detector,
    resampler: Option<FastFixedIn<f32>>,
    input_channels: u16,
    resample_buffer: Vec<f32>,
    vad_buffer: Vec<f32>,
    transcription_buffer: Vec<f32>,
}

impl VadProcessor {
    pub fn new(sample_rate: u32, channels: u16) -> Self {
        let detector = Detector::default();
        let resampler = if sample_rate != 16000 {
            let ratio = 16000.0 / sample_rate as f64;
            let input_chunk_size = (256.0 / ratio).round() as usize;
            
            Some(FastFixedIn::<f32>::new(
                ratio,
                2.0,
                rubato::PolynomialDegree::Septic,
                input_chunk_size,
                1,
            ).expect("Failed to create resampler"))
        } else {
            None
        };

        Self {
            detector,
            resampler,
            input_channels: channels,
            resample_buffer: Vec::new(),
            vad_buffer: Vec::new(),
            transcription_buffer: Vec::new(),
        }
    }

    pub fn process_samples(&mut self, samples: &[f32]) -> bool {
        let mut voice_detected = false;

        // 1. Mix to mono if necessary
        let mono_samples: Vec<f32> = if self.input_channels > 1 {
            samples.chunks_exact(self.input_channels as usize)
                .map(|chunk| chunk.iter().sum::<f32>() / self.input_channels as f32)
                .collect()
        } else {
            samples.to_vec()
        };

        // 2. Resample if necessary
        if let Some(resampler) = &mut self.resampler {
            self.resample_buffer.extend(mono_samples);
            let needed = resampler.input_frames_next();
            
            while self.resample_buffer.len() >= needed {
                let chunk = &self.resample_buffer[0..needed];
                let resampled = resampler.process(&[chunk], None).expect("Resampling failed");
                let frames = &resampled[0];
                self.vad_buffer.extend(frames);
                self.transcription_buffer.extend(frames);
                
                self.resample_buffer.drain(0..needed);
            }
        } else {
            self.vad_buffer.extend(&mono_samples);
            self.transcription_buffer.extend(mono_samples);
        };

        // 3. Buffer for VAD (requires exactly 256 samples)
        while self.vad_buffer.len() >= 256 {
            let frame = &self.vad_buffer[0..256];
            if self.detector.predict_f32(frame) > 0.5 {
                voice_detected = true;
            }
            self.vad_buffer.drain(0..256);
        }

        voice_detected
    }

    pub fn take_transcription_samples(&mut self) -> Vec<f32> {
        std::mem::take(&mut self.transcription_buffer)
    }
}
