# Development Guide: GNOME Dictation

This guide explains how to set up, build, and install the GNOME Dictation plugin and its associated Rust application for development.

## Prerequisites

Before starting, ensure you have the necessary dependencies installed.

### Fedora
```sh
sudo dnf install -y alsa-lib-devel gtk4-devel libadwaita-devel pkgconf-pkg-config dbus-devel gcc nodejs npm
```

### Ubuntu/Debian
```sh
sudo apt-get install -y libasound2-dev libgtk-4-dev libadwaita-1-dev pkg-config libdbus-1-dev gcc nodejs npm
```

## Build and Installation

The project consists of two main components: the Rust backend and the GNOME Shell extension (TypeScript).

### 1. Build the Rust Backend
The Rust application handles audio recording, transcription, and communication with the extension.

```sh
cargo build
```

### 2. Build the GNOME Extension
The extension is written in TypeScript and must be bundled into JavaScript before use.

```sh
cd gnome-extension
npm install
npm run build
cd ..
```

### 3. Install the Extension
Link the extension to your local GNOME Shell extensions directory and compile the settings schemas.

```sh
# Create the directory if it doesn't exist
mkdir -p ~/.local/share/gnome-shell/extensions/

# Link the extension (replace $(pwd) with the absolute path to the project root)
ln -s "$(pwd)/gnome-extension" ~/.local/share/gnome-shell/extensions/dictation@timharper

# Compile schemas
glib-compile-schemas gnome-extension/schemas/
```

### 4. Enable the Extension
To apply changes, you must restart GNOME Shell and then enable the extension.

- **On X11**: Press `Alt + F2`, type `r`, and press `Enter`.
- **On Wayland**: You must log out and log back in.
- **Enable via CLI**:
  ```sh
  gnome-extensions enable dictation@timharper
  ```

---

## Testing & Debugging

Testing GNOME extensions on your primary session can be disruptive. It is recommended to use a nested session for development.

### Nested GNOME Session
The project provides a script to launch a sandboxed GNOME session.

1. **Start the Nested Session**:
   ```bash
   ./test/scripts/child-gnome-session.sh start
   ```
   Wait about 10 seconds for the session to initialize.

2. **Set the DBus Context**:
   In every terminal where you run tests, set the DBus address to point to the nested session:
   ```bash
   export DBUS_SESSION_BUS_ADDRESS=$(cat /tmp/nested_dbus_address)
   ```

3. **Monitor Logs**:
   ```bash
   tail -f /tmp/nested_gnome.log
   ```

4. **Run Extension Tests**:
   The Rust app includes subcommands to verify extension functionality:
   - `cargo run -- extension update-menu`: Updates the tray icon and menu.
   - `cargo run -- extension listen`: Listens for menu clicks and the `<Super>d` shortcut.
   - `cargo run -- extension type "Hello World"`: Simulates typing.

5. **Stop the Session**:
   ```bash
   ./test/scripts/child-gnome-session.sh stop
   ```

### Debugging Extension Logs
To see logs from the extension in your host session:
```sh
journalctl -f -o cat /usr/bin/gnome-shell
```
Look for lines prefixed with `[dictation@timharper]`.

## Development Workflow

1.  Make changes to `gnome-extension/extension.ts`.
2.  Run `npm run build` in the `gnome-extension` directory.
3.  Restart GNOME Shell (or restart the nested session).
4.  Test your changes using the `cargo run -- extension` commands.
