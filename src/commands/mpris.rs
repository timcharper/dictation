use zbus::Connection;
use crate::mpris;

#[derive(clap::Subcommand)]
pub enum MprisCommands {
    /// Show playback status and metadata for all active MPRIS players
    Status,
    /// Send Play to the active player
    Play,
    /// Send Pause to the active player
    Pause,
}

pub async fn run(cmd: MprisCommands) {
    let conn = Connection::session().await.expect("Failed to connect to session bus");
    let client = mpris::MprisClient::new(conn);

    let players = client.find_players().await.expect("Failed to list MPRIS players");
    if players.is_empty() {
        println!("No MPRIS players found.");
        return;
    }

    match cmd {
        MprisCommands::Status => {
            for service in &players {
                let proxy = client.get_proxy(service).await.expect("Failed to get player proxy");
                let status = proxy.playback_status().await.unwrap_or_else(|_| "unknown".into());
                let metadata = proxy.metadata().await.unwrap_or_default();
                let title = mpris::string_field(&metadata, "xesam:title");
                let artist = mpris::string_field(&metadata, "xesam:artist");
                let album = mpris::string_field(&metadata, "xesam:album");
                let track_id = mpris::extract_track_id(&metadata);
                println!("Player: {}", service);
                println!("  Status:   {}", status);
                if !title.is_empty()  { println!("  Title:    {}", title); }
                if !artist.is_empty() { println!("  Artist:   {}", artist); }
                if !album.is_empty()  { println!("  Album:    {}", album); }
                println!("  Track ID: {}", track_id);
            }
        }
        MprisCommands::Pause => {
            let service = players.iter().next().expect("No players");
            let proxy = client.get_proxy(service).await.expect("Failed to get player proxy");
            proxy.pause().await.expect("Failed to pause");
            println!("Paused: {}", service);
        }
        MprisCommands::Play => {
            let service = players.iter().next().expect("No players");
            let proxy = client.get_proxy(service).await.expect("Failed to get player proxy");
            proxy.play().await.expect("Failed to play");
            println!("Playing: {}", service);
        }
    }
}
