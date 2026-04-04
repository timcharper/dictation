use atspi::AccessibilityConnection;
use atspi::connection::common::events::{object::StateChangedEvent, window::ActivateEvent, Event, WindowEvents};
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
        eprintln!("Failed to register for StateChanged events: {e}"); return;
    }
    if let Err(e) = connection.register_event::<ActivateEvent>().await {
        eprintln!("Failed to register for Window Activate events: {e}"); return;
    }

    let session_conn = match zbus::Connection::session().await {
        Ok(c) => c,
        Err(e) => { eprintln!("Failed to connect to session bus: {e}"); return; }
    };
    let extension_proxy = crate::extension_proxy::ExtensionProxy::new(&session_conn).await.ok();
    if extension_proxy.is_none() {
        eprintln!("Warning: extension not available — wm_class will show as ?");
    }

    println!("Watching AT-SPI focus + window events...");
    println!("Use wm_class values to configure accessibility_blacklist in dictation.toml\n");

    enum DrainMsg {
        Focus(atspi_common::ObjectRefOwned),
        WindowActivate(String),
    }

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<DrainMsg>();
    let drain_conn = connection.clone();
    tokio::spawn(async move {
        let mut stream = drain_conn.event_stream();
        while let Some(result) = stream.next().await {
            match result {
                Ok(Event::Object(atspi::connection::common::events::ObjectEvents::StateChanged(ev))) => {
                    if ev.state == atspi_common::State::Focused && ev.enabled {
                        let _ = tx.send(DrainMsg::Focus(ev.item));
                    }
                }
                Ok(Event::Window(WindowEvents::Activate(ev))) => {
                    let name = ev.item.name().map(|n| n.to_string()).unwrap_or_default();
                    let _ = tx.send(DrainMsg::WindowActivate(name));
                }
                _ => {}
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
                    Some(DrainMsg::Focus(item)) => {
                        let node = match item.as_accessible_proxy(connection.connection()).await {
                            Ok(n) => n,
                            Err(e) => { eprintln!("[focus] proxy error: {e}"); continue; }
                        };
                        let name = node.name().await.unwrap_or_default();
                        let role = node.get_role().await.map(|r| format!("{r:?}")).unwrap_or_default();
                        println!("Focus  → {role} \"{name}\"");
                        current = Some(item);
                    }
                    Some(DrainMsg::WindowActivate(name)) => {
                        println!("Window → \"{name}\"");
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

                let wm_class = match &extension_proxy {
                    Some(p) => p.get_focused_window_class().await.ok().unwrap_or_else(|| "?".to_string()),
                    None => "?".to_string(),
                };

                let text_info = async {
                    let proxies = node.proxies().await.ok()?;
                    let text_proxy = proxies.text().await.ok()?;
                    let offset = text_proxy.caret_offset().await.ok().filter(|&o| o >= 0)?;
                    let start = (offset - 100).max(0);
                    let text_before = text_proxy.get_text(start, offset).await.unwrap_or_default();
                    Some(format!("  context={text_before:?}"))
                }.await.unwrap_or_else(|| "  (no AT-SPI context)".to_string());

                println!("  [wm={wm_class}]{text_info}");
            }
        }
    }
}
