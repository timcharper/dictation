use zbus::Connection;
use crate::extension_proxy::ExtensionProxy;

#[derive(clap::Subcommand)]
pub enum ExtensionCommands {
    /// Type a string into the focused window
    Type { text: String },
    /// Get clipboard content
    GetClipboard,
    /// Set clipboard content
    SetClipboard { text: String },
    /// Raise the application window
    Raise,
    /// Update tray icon and menu
    UpdateMenu,
    /// Listen for extension events (shortcut, menu selection)
    Listen,
    /// Register a global shortcut
    RegisterShortcut { shortcut: String },
    /// Unregister the global shortcut
    UnregisterShortcut,
}

pub async fn run(cmd: ExtensionCommands) {
    let conn = Connection::session().await.expect("Failed to connect to session bus");
    let proxy = ExtensionProxy::new(&conn).await.expect("Failed to create extension proxy");

    match cmd {
        ExtensionCommands::Type { text } => {
            println!("Typing: '{}'", text);
            proxy.type_string(&text).await.expect("Failed to type string");
        }
        ExtensionCommands::GetClipboard => {
            let text = proxy.get_clipboard().await.expect("Failed to get clipboard");
            println!("Clipboard: '{}'", text);
        }
        ExtensionCommands::SetClipboard { text } => {
            println!("Setting clipboard to: '{}'", text);
            proxy.set_clipboard(&text).await.expect("Failed to set clipboard");
        }
        ExtensionCommands::Raise => {
            println!("Raising app...");
            proxy.raise_app().await.expect("Failed to raise app");
        }
        ExtensionCommands::UpdateMenu => {
            println!("Updating menu...");
            proxy.update("audio-input-microphone-symbolic", vec![
                ("test1".to_string(), "Test Item 1".to_string()),
                ("test2".to_string(), "Test Item 2".to_string()),
                ("quit".to_string(), "Quit".to_string()),
            ], "idle", "").await.expect("Failed to update menu");
        }
        ExtensionCommands::Listen => {
            println!("Listening for extension events. Press Ctrl+C to stop.");

            let mut menu_stream = proxy.receive_menu_item_selected().await.expect("Failed to receive menu signals");
            let mut shortcut_stream = proxy.receive_shortcut_pressed().await.expect("Failed to receive shortcut signals");

            loop {
                tokio::select! {
                    Some(signal) = tokio_stream::StreamExt::next(&mut menu_stream) => {
                        let args = signal.args().expect("Failed to parse signal args");
                        println!("Menu Item Selected: {}", args.id);
                    }
                    Some(_) = tokio_stream::StreamExt::next(&mut shortcut_stream) => {
                        println!("Shortcut Pressed!");
                    }
                }
            }
        }
        ExtensionCommands::RegisterShortcut { shortcut } => {
            println!("Registering shortcut: '{}'", shortcut);
            proxy.register_shortcut(&shortcut).await.expect("Failed to register shortcut");
        }
        ExtensionCommands::UnregisterShortcut => {
            println!("Unregistering shortcut...");
            proxy.unregister_shortcut().await.expect("Failed to unregister shortcut");
        }
    }
}
