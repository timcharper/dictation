use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream, StreamConfig};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use bytes::Bytes;

pub struct AudioRecorder {
    host: cpal::Host,
}

pub struct RecorderOutput {
    pub stream: Stream,
    pub audio_stream: ReceiverStream<Bytes>,
    pub config: StreamConfig,
}

impl AudioRecorder {
    pub fn new() -> Self {
        Self {
            host: cpal::default_host(),
        }
    }

    pub fn start_recording(&self) -> RecorderOutput {
        let device = self.host.default_input_device()
            .expect("Failed to find default input device");

        let config = device.default_input_config()
            .expect("Failed to get default input config");
        
        let stream_config: StreamConfig = config.clone().into();
        let (tx, rx) = mpsc::channel(100);

        // Capture samples in a separate thread/stream
        let stream = match config.sample_format() {
            SampleFormat::F32 => {
                device.build_input_stream(
                    &stream_config,
                    move |data: &[f32], _| {
                        let bytes = Bytes::copy_from_slice(bytemuck::cast_slice(data));
                        let _ = tx.blocking_send(bytes);
                    },
                    |err| eprintln!("Stream error: {:?}", err),
                    None,
                ).expect("Failed to build input stream")
            },
            _ => panic!("Unsupported sample format. Only F32 is supported for now."),
        };

        stream.play().expect("Failed to play stream");

        RecorderOutput {
            stream,
            audio_stream: ReceiverStream::new(rx),
            config: stream_config,
        }
    }
}
