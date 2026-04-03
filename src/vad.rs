use earshot::Detector;
use rubato::{Resampler, FastFixedIn};

pub struct VadProcessor {
    detector: Detector,
    resampler: Option<FastFixedIn<f32>>,
    input_channels: u16,
    resample_buffer: Vec<f32>,
    vad_buffer: Vec<f32>,
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
        }
    }

    pub fn process_samples(&mut self, samples: &[f32]) -> bool {
        let mono_samples: Vec<f32> = if self.input_channels > 1 {
            samples.chunks_exact(self.input_channels as usize)
                .map(|chunk| chunk.iter().sum::<f32>() / self.input_channels as f32)
                .collect()
        } else {
            samples.to_vec()
        };

        let mut voice_detected = false;

        let final_samples = if let Some(resampler) = &mut self.resampler {
            self.resample_buffer.extend(mono_samples);
            let needed = resampler.input_frames_next();
            let mut out = Vec::new();
            
            while self.resample_buffer.len() >= needed {
                let chunk: Vec<f32> = self.resample_buffer.drain(0..needed).collect();
                let resampled = resampler.process(&[chunk], None).expect("Resampling failed");
                out.extend(resampled[0].iter().copied());
            }
            out
        } else {
            mono_samples
        };

        self.vad_buffer.extend(final_samples);
        
        while self.vad_buffer.len() >= 256 {
            let frame: Vec<f32> = self.vad_buffer.drain(0..256).collect();
            // earshot predict_f32 returns a score (float)
            if self.detector.predict_f32(&frame) > 0.5 {
                voice_detected = true;
            }
        }

        voice_detected
    }
}
