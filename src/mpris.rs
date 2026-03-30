use std::collections::HashMap;
use zbus::Connection;
use zbus::zvariant::OwnedValue;

const MPRIS_PREFIX: &str = "org.mpris.MediaPlayer2.";

#[zbus::proxy(
    interface = "org.mpris.MediaPlayer2.Player",
    default_path = "/org/mpris/MediaPlayer2"
)]
pub trait MediaPlayer2Player {
    async fn play(&self) -> zbus::Result<()>;
    async fn pause(&self) -> zbus::Result<()>;

    #[zbus(property)]
    fn playback_status(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn metadata(&self) -> zbus::Result<HashMap<String, OwnedValue>>;
}

pub struct MprisClient {
    conn: Connection,
}

/// Snapshot of a player at the moment it was paused — used to check if the
/// same track is still playing before resuming.
pub struct PlayerState {
    pub service: String,
    pub track_id: String,
}

impl MprisClient {
    pub fn new(conn: Connection) -> Self {
        MprisClient { conn }
    }

    pub async fn find_players(&self) -> zbus::Result<Vec<String>> {
        let dbus = zbus::fdo::DBusProxy::new(&self.conn).await?;
        let names = dbus.list_names().await?;
        Ok(names
            .into_iter()
            .filter(|n| n.starts_with(MPRIS_PREFIX))
            .map(|n| n.to_string())
            .collect())
    }

    pub async fn get_proxy<'a>(&'a self, service: &str) -> zbus::Result<MediaPlayer2PlayerProxy<'a>> {
        MediaPlayer2PlayerProxy::builder(&self.conn)
            .destination(service.to_owned())?
            .build()
            .await
    }
}

/// Extract a stable track identifier from MPRIS metadata.
/// Prefers `mpris:trackid`; falls back to `xesam:title` + first `xesam:artist`.
pub fn extract_track_id(metadata: &HashMap<String, OwnedValue>) -> String {
    if let Some(v) = metadata.get("mpris:trackid") {
        if let Ok(s) = String::try_from(v.clone()) {
            if !s.is_empty() && s != "/org/mpris/MediaPlayer2/TrackList/NoTrack" {
                return s;
            }
        }
    }
    let title = string_field(metadata, "xesam:title");
    let artist = array_first_string(metadata, "xesam:artist");
    format!("{}|{}", title, artist)
}

pub fn string_field(metadata: &HashMap<String, OwnedValue>, key: &str) -> String {
    metadata
        .get(key)
        .and_then(|v| String::try_from(v.clone()).ok())
        .unwrap_or_default()
}

/// Try to read a field that is an array of strings (e.g. xesam:artist) and return the first element.
fn array_first_string(metadata: &HashMap<String, OwnedValue>, key: &str) -> String {
    metadata
        .get(key)
        .and_then(|v| {
            // OwnedValue wraps an Array; try to get the inner Vec<String>
            let arr: Result<Vec<String>, _> = v.clone().try_into();
            arr.ok()
        })
        .and_then(|v| v.into_iter().next())
        .unwrap_or_default()
}
