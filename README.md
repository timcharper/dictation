# Dictation

A dictation application for GNOME using Rust, GTK4, Libadwaita, Whisper, and Ollama.

## Dependencies

On Fedora, install the required development libraries:

```sh
sudo dnf install -y alsa-lib-devel gtk4-devel libadwaita-devel pkgconf-pkg-config dbus-devel gcc
```

## Build

```sh
cargo build
```

## GNOME Extension

To install the extension on your host system:

1. Link the extension:
   ```sh
   mkdir -p ~/.local/share/gnome-shell/extensions/
   ln -s $(pwd)/gnome-extension ~/.local/share/gnome-shell/extensions/dictation@timharper
   ```
2. Compile schemas:
   ```sh
   glib-compile-schemas gnome-extension/schemas/
   ```
3. Restart GNOME Shell and enable `dictation@timharper`.

## Testing

The application provides several subcommands for testing individual components.

### 1. Microphone Test
Records 5 seconds of audio and saves it to a WAV file in `/tmp`.
```sh
cargo run -- microphone
```

### 2. Transcription Test
Transcribes a WAV file using the configured Whisper backend.
```sh
cargo run -- transcribe test/fixtures/jfk.wav
```

### 3. Accessibility (AT-SPI) Test
Prints the visible text and focused context of the active window.
```sh
cargo run -- accessibility
```

### 4. GNOME Extension Interactivity
Use the test script to launch a nested session first:
```sh
./test/scripts/child-gnome-session.sh start
export DBUS_SESSION_BUS_ADDRESS=$(cat /tmp/nested_dbus_address)
```

Then run interactivity tests:
*   **Update Menu**: `cargo run -- extension update-menu`
*   **Type Text**: `cargo run -- extension type "Hello World"`
*   **Get Clipboard**: `cargo run -- extension get-clipboard`
*   **Listen for Events**: `cargo run -- extension listen` (Detects shortcut and menu clicks)
