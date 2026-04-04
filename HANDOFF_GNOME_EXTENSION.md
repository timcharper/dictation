# Handoff: GNOME Extension Verification

This document outlines the architecture, setup, and verification steps for the `dictation` GNOME extension integration.

## Architecture
- **Extension (`gnome-extension/`)**: Written in TypeScript (`extension.ts`), bundled to `extension.js`. It exports a DBus interface `com.timcharper.dictation.Extension` at `/com/timcharper/dictation/Extension`.
- **Rust Application**: Communicates with the extension via `src/extension_proxy.rs` using the `zbus` crate.
- **Communication**:
  - **Methods**: `Update` (Tray/Menu), `RaiseApp`, `GetClipboard`, `SetClipboard`, `TypeString`, `RegisterShortcut`.
  - **Signals**: `MenuItemSelected`, `ShortcutPressed`.

## Setup Verification Environment

Since testing GNOME extensions can interfere with the host session and hardware acceleration (Nvidia) can be problematic, use the nested session script:

1. **Start the Nested Session**:
   ```bash
   ./test/scripts/child-gnome-session.sh start
   ```
   *Wait ~10 seconds for initialization.*

2. **Set the DBus Context**:
   Every subsequent test command **must** use the nested D-Bus address:
   ```bash
   export DBUS_SESSION_BUS_ADDRESS=$(cat /tmp/nested_dbus_address)
   ```

3. **Monitor Logs**:
   ```bash
   tail -f /tmp/nested_gnome.log
   ```
   Look for `[dictation@timcharper.com] Enabled`.

## Verification Tasks

The following subcommands in the Rust app need to be verified against the nested session:

| Command | Action | Expected Outcome |
| :--- | :--- | :--- |
| `cargo run -- extension update-menu` | Updates tray icon and menu items. | A microphone icon should appear in the nested top bar with "Test Item 1", "Test Item 2", and "Quit". |
| `cargo run -- extension listen` | Blocks and listens for signals. | 1. Click a menu item: Terminal should print `Menu Item Selected: <id>`. <br> 2. Press `<Super>d`: Terminal should print `Shortcut Pressed!`. |
| `cargo run -- extension type "Test"` | Simulates typing. | Open a text entry in the nested session (if possible) or check logs for keyval activity. |
| `cargo run -- extension get-clipboard` | Reads nested clipboard. | Should return the current text in the nested session's clipboard. |
| `cargo run -- extension set-clipboard "Hi"` | Writes to nested clipboard. | Setting the text should make it available for pasting within the nested session. |
| `cargo run -- extension raise` | Raises the app. | Should attempt to focus the `com.timcharper.dictation` window if one is open in the nested session. |

## Key Files
- `gnome-extension/extension.ts`: Source logic for the extension.
- `src/extension_proxy.rs`: Rust DBus proxy definitions.
- `src/main.rs`: CLI implementation for the verification commands.
- `test/scripts/child-gnome-session.sh`: Sandbox orchestration script.

## Cleanup
```bash
./test/scripts/child-gnome-session.sh stop
```
