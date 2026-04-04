use std::path::PathBuf;
use tokio::fs::File;
use tokio_util::io::ReaderStream;

pub async fn run(path: PathBuf) {
    println!("Transcribing file: {:?}", path);

    let config = crate::config::Config::load();
    let transcriber = crate::transcriber_factory::create_transcriber(&config.backend, None);

    println!("Using configured backend");

    let file = File::open(path).await.expect("Failed to open WAV file");
    let stream = ReaderStream::new(file);
    let bytes_stream = tokio_stream::StreamExt::filter_map(stream, |res| res.ok());

    let mut transcription_stream = match transcriber.stream_transcription(Box::pin(bytes_stream)).await {
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
