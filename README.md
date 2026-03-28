# Dictation

A dictation application for GNOME using Rust, GTK4, Libadwaita, Whisper, and Ollama.

## Dependencies

On Fedora, install the required development libraries:

```sh
sudo dnf install -y alsa-lib-devel gtk4-devel libadwaita-devel pkgconf-pkg-config dbus-devel gcc
```

## Build

```sh
cargo build --release
```
