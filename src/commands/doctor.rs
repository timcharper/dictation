use cpal::traits::{DeviceTrait, HostTrait};
use reqwest::Client;
use std::path::Path;
use std::time::Duration;
use crate::config::{BackendConfig, Config};

struct Check {
    label: String,
    status: Status,
    detail: Option<String>,
}

enum Status {
    Ok,
    Warn,
    Fail,
}

impl Check {
    fn ok(label: impl Into<String>) -> Self {
        Self { label: label.into(), status: Status::Ok, detail: None }
    }

    fn ok_detail(label: impl Into<String>, detail: impl Into<String>) -> Self {
        Self { label: label.into(), status: Status::Ok, detail: Some(detail.into()) }
    }

    fn warn(label: impl Into<String>, detail: impl Into<String>) -> Self {
        Self { label: label.into(), status: Status::Warn, detail: Some(detail.into()) }
    }

    fn fail(label: impl Into<String>, detail: impl Into<String>) -> Self {
        Self { label: label.into(), status: Status::Fail, detail: Some(detail.into()) }
    }

    fn print(&self) {
        let (icon, color) = match self.status {
            Status::Ok   => ("✓", "\x1b[32m"),
            Status::Warn => ("!", "\x1b[33m"),
            Status::Fail => ("✗", "\x1b[31m"),
        };
        let reset = "\x1b[0m";
        let dim = "\x1b[2m";

        print!("  {color}{icon}{reset} {}", self.label);
        if let Some(d) = &self.detail {
            print!("  {dim}{d}{reset}");
        }
        println!();
    }
}

pub async fn run() {
    let config = Config::load();
    let mut checks: Vec<Check> = Vec::new();
    let mut any_fail = false;

    // ── Configuration ────────────────────────────────────────────────────────

    println!("\n\x1b[1mConfiguration\x1b[0m");

    checks.push(Check::ok_detail(
        "Config file",
        Config::path().display().to_string(),
    ));

    // Shortcut
    if config.shortcut.is_empty() {
        checks.push(Check::warn("Shortcut", "no shortcut configured — dictation can only be triggered programmatically"));
    } else {
        checks.push(Check::ok_detail("Shortcut", config.shortcut.clone()));
    }

    // Sound files
    if let Some(path) = &config.sound.start_sound {
        if !path.is_empty() && !Path::new(path).exists() {
            checks.push(Check::warn("Start sound", format!("file not found: {path}")));
        } else if !path.is_empty() {
            checks.push(Check::ok_detail("Start sound", path.clone()));
        }
    }
    if let Some(path) = &config.sound.end_sound {
        if !path.is_empty() && !Path::new(path).exists() {
            checks.push(Check::warn("End sound", format!("file not found: {path}")));
        } else if !path.is_empty() {
            checks.push(Check::ok_detail("End sound", path.clone()));
        }
    }

    for c in checks.drain(..) {
        if matches!(c.status, Status::Fail) { any_fail = true; }
        c.print();
    }

    // ── Microphone ───────────────────────────────────────────────────────────

    println!("\n\x1b[1mMicrophone\x1b[0m");

    let host = cpal::default_host();
    match host.default_input_device() {
        None => Check::fail("Default input device", "no microphone found").print(),
        Some(device) => {
            let name = device.name().unwrap_or_else(|_| "<unknown>".to_string());
            match device.default_input_config() {
                Ok(cfg) => Check::ok_detail(
                    "Default input device",
                    format!("{name}  {}Hz  {}ch", cfg.sample_rate().0, cfg.channels()),
                ).print(),
                Err(e) => {
                    any_fail = true;
                    Check::fail("Default input device", format!("{name} — config error: {e}")).print();
                }
            }
        }
    }

    // ── Whisper backend ──────────────────────────────────────────────────────

    println!("\n\x1b[1mTranscription Backend\x1b[0m");

    let BackendConfig::WhisperCpp { url } = &config.backend;

    // Build the inference URL the same way WhisperClient does
    let mut inference_url = url.clone();
    if !inference_url.contains("/inference") {
        if !inference_url.ends_with('/') { inference_url.push('/'); }
        inference_url.push_str("inference");
    }

    Check::ok_detail("Whisper server URL", url.clone()).print();

    // Load the bundled JFK fixture (embedded at compile time so the binary
    // is self-contained and `doctor` works regardless of CWD).
    let wav_bytes: &[u8] = include_bytes!("../../test/fixtures/jfk.wav");

    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("Failed to build HTTP client");

    match client
        .post(&inference_url)
        .header("Content-Type", "audio/wav")
        .body(wav_bytes)
        .send()
        .await
    {
        Err(e) => {
            any_fail = true;
            Check::fail("Whisper /inference", format!("request failed: {e}")).print();
        }
        Ok(resp) => {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            if status.is_success() {
                // Try to pull the text field out for a sanity preview
                let preview = serde_json::from_str::<serde_json::Value>(&body)
                    .ok()
                    .and_then(|v| v["text"].as_str().map(|s| format!("\"{}\"", s.trim())))
                    .unwrap_or_else(|| body.chars().take(80).collect());
                Check::ok_detail("Whisper /inference", format!("{status}  {preview}")).print();
            } else {
                any_fail = true;
                let snippet: String = body.chars().take(120).collect();
                Check::fail("Whisper /inference", format!("{status}  {snippet}")).print();
            }
        }
    }

    // ── GNOME Extension ──────────────────────────────────────────────────────

    println!("\n\x1b[1mGNOME Extension\x1b[0m");

    match zbus::Connection::session().await {
        Err(e) => {
            any_fail = true;
            Check::fail("D-Bus session bus", format!("could not connect: {e}")).print();
        }
        Ok(conn) => {
            Check::ok("D-Bus session bus").print();

            let dbus_proxy = zbus::fdo::DBusProxy::new(&conn).await
                .expect("Failed to create DBus proxy");
            let ext_name: zbus::names::WellKnownName =
                "com.timcharper.dictation.Extension".try_into().unwrap();
            match dbus_proxy.get_name_owner(ext_name.into()).await {
                Ok(owner) => Check::ok_detail(
                    "Dictation extension",
                    format!("com.timcharper.dictation.Extension is running ({owner})"),
                ).print(),
                Err(_) => {
                    any_fail = true;
                    Check::fail(
                        "Dictation extension",
                        "com.timcharper.dictation.Extension not found — is the GNOME extension enabled?",
                    ).print();
                }
            }
        }
    }

    // ── Accessibility (AT-SPI) ───────────────────────────────────────────────

    println!("\n\x1b[1mAccessibility (AT-SPI)\x1b[0m");

    match zbus::Connection::session().await {
        Err(_) => {} // already reported above
        Ok(conn) => {
            let result = async {
                let props = zbus::fdo::PropertiesProxy::builder(&conn)
                    .destination("org.a11y.Bus")?
                    .path("/org/a11y/bus")?
                    .build()
                    .await?;
                let iface = zbus::names::InterfaceName::try_from("org.a11y.Status")?;
                let value = props.get(iface, "IsEnabled").await?;
                let enabled: bool = bool::try_from(value)?;
                Ok::<bool, Box<dyn std::error::Error + Send + Sync>>(enabled)
            }.await;

            match result {
                Ok(true) => Check::ok("Accessibility framework enabled").print(),
                Ok(false) => {
                    Check::warn(
                        "Accessibility framework disabled",
                        "many apps won't broadcast AT-SPI events — context will be unavailable\n\
                         \x1b[2m       Enable with:  gsettings set org.gnome.desktop.interface toolkit-accessibility true\n\
                         \x1b[2m       Note: apps already running (e.g. Firefox) must be restarted to pick up the change.\x1b[0m",
                    ).print();
                }
                Err(e) => {
                    Check::warn("Accessibility framework", format!("could not query status: {e}")).print();
                }
            }
        }
    }

    // ── Summary ──────────────────────────────────────────────────────────────

    println!();
    if any_fail {
        println!("\x1b[31mSome checks failed.\x1b[0m Fix the issues above and re-run `dictation doctor`.");
        std::process::exit(1);
    } else {
        println!("\x1b[32mAll checks passed.\x1b[0m");
    }
}
