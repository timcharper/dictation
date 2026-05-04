use audiopus::{coder::Encoder, Application, Channels, SampleRate};
use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use ogg::PacketWriteEndInfo;
use std::pin::Pin;

pub struct OpusOggEncoder {
    encoder: Encoder,
    buffer: Vec<f32>,
    frame_size: usize,
}

impl OpusOggEncoder {
    pub fn new() -> Self {
        let encoder = Encoder::new(SampleRate::Hz16000, Channels::Mono, Application::Voip)
            .expect("Failed to create Opus encoder");
        
        // 20ms frame at 16kHz = 320 samples
        let frame_size = 320;

        Self {
            encoder,
            buffer: Vec::with_capacity(frame_size),
            frame_size,
        }
    }

    pub fn encode_stream<S>(
        self,
        pcm_stream: S,
    ) -> Pin<Box<dyn Stream<Item = Result<Bytes, String>> + Send>>
    where
        S: Stream<Item = Bytes> + Send + Unpin + 'static,
    {
        let encoder = self;
        let stream_serial = rand::random::<u32>();
        let packet_count = 0u64;
        let abs_granule_pos = 0u64;

        let stream = futures_util::stream::unfold(
            (pcm_stream, encoder, false, stream_serial, packet_count, abs_granule_pos),
            |(mut pcm_stream, mut encoder, mut finished, stream_serial, mut packet_count, mut abs_granule_pos)| async move {
                if finished {
                    return None;
                }

                let mut ogg_output = Vec::new();
                let mut ogg_writer = ogg::PacketWriter::new(&mut ogg_output);

                // 1. Write headers if this is the start
                if packet_count == 0 {
                    // OpusHead packet
                    let mut head = Vec::new();
                    head.extend_from_slice(b"OpusHead");
                    head.push(1); // version
                    head.push(1); // channels
                    head.extend_from_slice(&[0, 0]); // pre-skip
                    head.extend_from_slice(&16000u32.to_le_bytes()); // original sample rate
                    head.extend_from_slice(&[0, 0]); // gain
                    head.push(0); // mapping family
                    
                    ogg_writer.write_packet(head, stream_serial, PacketWriteEndInfo::EndPage, 0).unwrap();
                    
                    // OpusTags packet
                    let mut tags = Vec::new();
                    tags.extend_from_slice(b"OpusTags");
                    tags.extend_from_slice(&8u32.to_le_bytes()); // vendor length
                    tags.extend_from_slice(b"dictation");
                    tags.extend_from_slice(&0u32.to_le_bytes()); // user comment list length
                    
                    ogg_writer.write_packet(tags, stream_serial, PacketWriteEndInfo::EndPage, 0).unwrap();
                    packet_count = 2;
                }

                loop {
                    // Try to encode a frame from the buffer
                    if encoder.buffer.len() >= encoder.frame_size {
                        let frame: Vec<f32> = encoder.buffer.drain(0..encoder.frame_size).collect();
                        let mut opus_packet = vec![0u8; 1275]; // Max recommended packet size
                        
                        match encoder.encoder.encode_float(&frame, &mut opus_packet) {
                            Ok(size) => {
                                opus_packet.truncate(size);
                                abs_granule_pos += encoder.frame_size as u64;
                                
                                ogg_writer.write_packet(
                                    opus_packet,
                                    stream_serial,
                                    PacketWriteEndInfo::EndPage,
                                    abs_granule_pos
                                ).unwrap();
                                
                                packet_count += 1;
                                // We yielded a page, return the current output
                                return Some((Ok(Bytes::from(ogg_output)), (pcm_stream, encoder, false, stream_serial, packet_count, abs_granule_pos)));
                            }
                            Err(e) => {
                                return Some((Err(format!("Opus encoding error: {:?}", e)), (pcm_stream, encoder, true, stream_serial, packet_count, abs_granule_pos)));
                            }
                        }
                    }

                    // Need more data
                    match pcm_stream.next().await {
                        Some(chunk) => {
                            let samples: &[f32] = bytemuck::cast_slice(&chunk);
                            encoder.buffer.extend_from_slice(samples);
                        }
                        None => {
                            if !encoder.buffer.is_empty() {
                                encoder.buffer.resize(encoder.frame_size, 0.0);
                                finished = true;
                                // Continue loop to process the padded final frame
                            } else {
                                if ogg_output.is_empty() {
                                    return None;
                                } else {
                                    return Some((Ok(Bytes::from(ogg_output)), (pcm_stream, encoder, true, stream_serial, packet_count, abs_granule_pos)));
                                }
                            }
                        }
                    }
                }
            },
        );

        Box::pin(stream)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::stream;

    #[tokio::test]
    async fn test_opus_ogg_encoding() {
        let encoder = OpusOggEncoder::new();
        // Create 1 second of silence (16000 samples)
        let samples = vec![0.0f32; 16000];
        let bytes = Bytes::from(bytemuck::cast_slice::<f32, u8>(&samples).to_vec());
        let pcm_stream = stream::once(async move { bytes });

        let mut encoded_stream = encoder.encode_stream(Box::pin(pcm_stream));
        let mut total_bytes = 0;
        let mut page_count = 0;

        while let Some(res) = encoded_stream.next().await {
            match res {
                Ok(bytes) => {
                    total_bytes += bytes.len();
                    page_count += 1;
                }
                Err(e) => panic!("Encoding error: {}", e),
            }
        }

        println!("Encoded {} bytes in {} Ogg pages", total_bytes, page_count);
        assert!(total_bytes > 0);
        assert!(page_count >= 3); // 2 header pages + at least 1 data page
    }
}
