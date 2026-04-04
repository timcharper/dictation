use atspi::AccessibilityConnection;
use atspi::connection::common::events::{object::StateChangedEvent, Event};
use atspi::proxy::accessible::ObjectRefExt;
use atspi::proxy::proxy_ext::ProxyExt;
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

    // Spawn a dedicated drain task that does nothing but filter and forward.
    // This keeps the zbus broadcast channel fully drained regardless of how
    // fast the processing loop runs.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<atspi_common::ObjectRefOwned>();
    let drain_conn = connection.clone();
    tokio::spawn(async move {
        let mut stream = drain_conn.event_stream();
        while let Some(result) = stream.next().await {
            if let Ok(Event::Object(atspi::connection::common::events::ObjectEvents::StateChanged(ev))) = result {
                if ev.state == atspi_common::State::Focused && ev.enabled {
                    let _ = tx.send(ev.item);
                }
            }
        }
        eprintln!("[watcher] event stream ended");
    });

    let mut current: Option<atspi_common::ObjectRefOwned> = None;
    let mut poll = tokio::time::interval(std::time::Duration::from_millis(500));
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            item = rx.recv() => {
                match item {
                    Some(item) => {
                        // Print focus change immediately
                        let node = match item.as_accessible_proxy(connection.connection()).await {
                            Ok(n) => n,
                            Err(e) => { eprintln!("[focus] proxy error: {e}"); continue; }
                        };
                        let name = node.name().await.unwrap_or_default();
                        let role = node.get_role().await.map(|r| format!("{r:?}")).unwrap_or_default();
                        println!("Focus → {role} \"{name}\"");
                        current = Some(item);
                    }
                    None => break,
                }
            }
            _ = poll.tick() => {
                let Some(ref item) = current else { continue };

                let node = match item.as_accessible_proxy(connection.connection()).await {
                    Ok(n) => n,
                    Err(_) => continue,
                };
                let proxies = match node.proxies().await {
                    Ok(p) => p,
                    Err(_) => continue,
                };
                match proxies.text().await {
                    Ok(text_proxy) => {
                        let offset = text_proxy.caret_offset().await.unwrap_or(-1);
                        if offset >= 0 {
                            let start = (offset - 100).max(0);
                            let text_before = text_proxy.get_text(start, offset).await.unwrap_or_default();
                            println!("  poll  offset={offset}  context={text_before:?}");
                        }
                    }
                    Err(_) => {}
                }
            }
        }
    }
}
