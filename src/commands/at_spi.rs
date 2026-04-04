use atspi::AccessibilityConnection;
use atspi::connection::common::events::{object::StateChangedEvent, Event};
use atspi::proxy::accessible::ObjectRefExt;
use atspi::proxy::proxy_ext::ProxyExt;
use atspi_common::State;
use futures_lite::StreamExt;

pub async fn snapshot() {
    println!("Initializing AT-SPI...");
    let manager = match crate::accessibility::AccessibilityManager::new().await {
        Ok(m) => m,
        Err(e) => {
            eprintln!("Failed to initialize AT-SPI: {:?}", e);
            return;
        }
    };

    println!("\nGrabbing visible text of active window...");
    match manager.get_visible_text().await {
        Ok(texts) => {
            for (i, text) in texts.iter().enumerate() {
                println!("VISIBLE {}: \"{}\"", i + 1, text);
            }
        }
        Err(e) => eprintln!("Error getting visible text: {:?}", e),
    }

    println!("\nGrabbing focused context (ancestors included)...");
    match manager.get_focused_context().await {
        Ok(texts) => {
            if texts.is_empty() {
                println!("No focused context found.");
            } else {
                for (i, text) in texts.iter().enumerate() {
                    println!("CONTEXT {}: \"{}\"", i + 1, text);
                }
            }
        }
        Err(e) => eprintln!("Error getting focused context: {:?}", e),
    }

    println!("\nGrabbing cursor info...");
    match manager.get_cursor_info().await {
        Ok(Some(info)) => {
            println!("Cursor offset: {}", info._offset);
            println!("Text before cursor: \"{}\"", info.text_before);
        }
        Ok(None) => println!("No cursor found in focused element."),
        Err(e) => eprintln!("Error getting cursor info: {:?}", e),
    }
}

pub async fn watcher() {
    let connection = match AccessibilityConnection::new().await {
        Ok(c) => c,
        Err(e) => { eprintln!("Failed to connect to AT-SPI: {e}"); return; }
    };

    if let Err(e) = connection.register_event::<StateChangedEvent>().await {
        eprintln!("Failed to register for StateChanged events: {e}");
        return;
    }

    println!("Watching AT-SPI focus events. Move focus to see cursor context...\n");

    let mut event_stream = connection.event_stream();
    while let Some(result) = event_stream.next().await {
        let ev = match result {
            Ok(Event::Object(atspi::connection::common::events::ObjectEvents::StateChanged(ev))) => ev,
            _ => continue,
        };

        if ev.state != State::Focused || !ev.enabled {
            continue;
        }

        let node = match ev.item.as_accessible_proxy(connection.connection()).await {
            Ok(n) => n,
            Err(e) => { eprintln!("[focus] Failed to get proxy: {e}"); continue; }
        };

        let name = node.name().await.unwrap_or_default();
        let role = node.get_role().await.map(|r| format!("{r:?}")).unwrap_or_default();

        let proxies = match node.proxies().await {
            Ok(p) => p,
            Err(_) => {
                println!("Focus → {role} \"{name}\" (no text interface)");
                continue;
            }
        };

        match proxies.text().await {
            Ok(text_proxy) => {
                let offset = text_proxy.caret_offset().await.unwrap_or(-1);
                if offset >= 0 {
                    let start = (offset - 100).max(0);
                    let text_before = text_proxy.get_text(start, offset).await.unwrap_or_default();
                    println!("Focus → {role} \"{name}\"  offset={offset}  context={text_before:?}");
                } else {
                    println!("Focus → {role} \"{name}\"  (no caret)");
                }
            }
            Err(_) => {
                println!("Focus → {role} \"{name}\" (no text interface)");
            }
        }
    }
}
