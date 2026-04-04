# Agent Instructions for Testing GNOME Extension

To test the GNOME extension in an isolated, nested environment (avoiding conflicts with your primary session and handling Nvidia driver limitations), follow these steps:

## 1. Launch the Nested Session

Use the `test/scripts/child-gnome-session.sh` script. This script creates an isolated sandbox, packages the extension, and launches a nested GNOME Shell using software rendering.

```sh
./test/scripts/child-gnome-session.sh start
```

- **Wait**: It takes about 8-10 seconds to fully initialize.
- **Status**: Check if it's running with `./test/scripts/child-gnome-session.sh status`.
- **Log Monitoring**: Monitor logs with `tail -f /tmp/nested_gnome.log`. Look for `[dictation@timcharper.com] Enabled` to confirm success.
- **Verification**: You can also verify by listing enabled extensions:
  ```sh
  export DBUS_SESSION_BUS_ADDRESS=$(cat /tmp/nested_dbus_address)
  gnome-extensions list --enabled
  ```
- **D-Bus Address**: The D-Bus address for the nested session is saved to `/tmp/nested_dbus_address`.

## 2. Interact with the Extension from Rust

When running the Rust application subcommands to test the extension, you **must** point it to the nested D-Bus session:

```sh
export DBUS_SESSION_BUS_ADDRESS=$(cat /tmp/nested_dbus_address)
cargo run -- extension update-menu
cargo run -- extension listen
```

## 3. Stopping the Session

```sh
./test/scripts/child-gnome-session.sh stop
```
