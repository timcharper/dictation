use zbus::Connection;

pub async fn run() {
    let conn = match Connection::session().await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to connect to session bus: {}", e);
            return;
        }
    };

    println!("Sending trigger to dictation daemon...");
    let result = conn.call_method(
        Some("com.timcharper.dictation.Daemon"),
        "/com/timcharper/dictation/Daemon",
        Some("com.timcharper.dictation.Daemon"),
        "Trigger",
        &(),
    ).await;

    match result {
        Ok(_) => println!("Trigger sent successfully."),
        Err(e) => eprintln!("Failed to send trigger: {}. Is the daemon running?", e),
    }
}
