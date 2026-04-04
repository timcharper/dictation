# Dictation

A voice dictation application for GNOME. Press a shortcut, speak, release — your words are typed at the cursor.

Built with Rust, GTK4, Libadwaita, and [whisper.cpp](https://github.com/timcharper/whisper.cpp).

---

## Supported Backends

### Whisper (whisper.cpp HTTP server)

The only currently supported transcription backend is a local [whisper.cpp](https://github.com/timcharper/whisper.cpp) HTTP server.

> **Important**: Use [timcharper/whisper.cpp](https://github.com/timcharper/whisper.cpp) — this fork includes prompt-forwarding support that allows Dictation to pass surrounding text as context to improve transcription accuracy.

Build and run the server by following the [whisper.cpp build instructions](https://github.com/timcharper/whisper.cpp). Once built, run:

```sh
whisper-server \
  -m /path/to/models/ggml-large-v3.bin \
  -vm /path/to/models/ggml-silero-v6.2.0.bin \
  -t 8 \
  --host 127.0.0.1 \
  --port 58080
```

Download models from [Hugging Face](https://huggingface.co/ggerganov/whisper.cpp).

---

## Installation

### 1. Dependencies

On Fedora:

```sh
sudo dnf install -y alsa-lib-devel gtk4-devel libadwaita-devel pkgconf-pkg-config dbus-devel gcc
```

### 2. Build

```sh
cargo build --release
```

### 3. GNOME Extension

The GNOME Shell extension provides the tray icon, keyboard shortcut handling, and text injection.

```sh
# Link the extension into GNOME Shell's extension directory
mkdir -p ~/.local/share/gnome-shell/extensions/
ln -s "$(pwd)/gnome-extension" ~/.local/share/gnome-shell/extensions/dictation@timcharper.com

# Compile GSettings schemas
glib-compile-schemas gnome-extension/schemas/
```

Then restart GNOME Shell (`Alt+F2` → `r` on X11, or log out and back in on Wayland) and enable the extension:

```sh
gnome-extensions enable dictation@timcharper.com
```

### 4. Configuration

Launch the settings UI:

```sh
cargo run -- settings   # or ./target/release/dictation
```

Set your Whisper server URL (default: `http://localhost:58080`) and configure a keyboard shortcut.

### 5. Verify with Doctor

Once everything is set up, run the health check to confirm all components are working:

```sh
./target/release/dictation doctor
```

This checks your microphone, the Whisper backend (by sending a real test audio clip), and that the GNOME extension is running. Fix any reported issues before proceeding.

---

## Running as a System Service

To have Dictation and Whisper start automatically at login, install systemd user services.

### whisper.service

Create `~/.config/systemd/user/whisper-service.service`:

```ini
[Unit]
Description=whisper.cpp server
After=network.target

[Service]
ExecStart=/path/to/whisper.cpp/build/bin/whisper-server \
    -m /path/to/models/ggml-large-v3.bin \
    -vm /path/to/models/ggml-silero-v6.2.0.bin \
    -t 8 \
    --host 127.0.0.1 \
    --port 58080
StandardError=journal
Restart=on-failure

[Install]
WantedBy=default.target
```

### dictation.service

Create `~/.config/systemd/user/dictation.service`:

```ini
[Unit]
Description=Dictation daemon
After=graphical-session.target

[Service]
ExecStartPre=/usr/bin/gdbus wait --session com.timcharper.dictation.Extension --timeout 30
ExecStart=/path/to/dictation daemon
Restart=on-failure
RestartSec=3

[Install]
WantedBy=graphical-session.target
```

### Enable and start

```sh
systemctl --user daemon-reload
systemctl --user enable --now whisper-service.service
systemctl --user enable --now dictation.service
```

---

## Development & Testing

The application provides subcommands for testing individual components.

### Microphone
Records 5 seconds of audio and saves it to a WAV file in `/tmp`.
```sh
cargo run -- microphone
```

### Transcription
Transcribes a WAV file using the configured backend.
```sh
cargo run -- transcribe test/fixtures/jfk.wav
```

### Accessibility (AT-SPI)
Prints the visible text and focused context of the active window.
```sh
cargo run -- at-spi
```

### AT-SPI Watcher
Watches focus events in real time and prints cursor context as focus changes.
```sh
cargo run -- at-spi-watcher
```

### GNOME Extension Integration
Launch a nested GNOME session for safe testing:
```sh
./test/scripts/child-gnome-session.sh start
export DBUS_SESSION_BUS_ADDRESS=$(cat /tmp/nested_dbus_address)
```

Then run extension commands:
```sh
cargo run -- extension update-menu
cargo run -- extension type "Hello World"
cargo run -- extension get-clipboard
cargo run -- extension listen   # blocks; detects shortcut and menu clicks
```
