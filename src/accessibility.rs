use atspi::proxy::accessible::AccessibleProxy;
use atspi::AccessibilityConnection;
use atspi::proxy::proxy_ext::ProxyExt;
use atspi::proxy::accessible::ObjectRefExt;
use atspi_common::{State, ObjectRefOwned};
use atspi::connection::common::events::{object::StateChangedEvent, window::ActivateEvent, Event, WindowEvents};
use std::error::Error;
use std::sync::{Arc, Mutex};
use futures_lite::StreamExt;
use tokio::sync::watch;

#[derive(Debug)]
pub struct CursorInfo {
    pub _offset: i32,
    pub text_before: String,
}

pub struct AccessibilityManager {
    connection: AccessibilityConnection,
    focused: Arc<Mutex<Option<ObjectRefOwned>>>,
    pub focus_receiver: watch::Receiver<()>,
    _event_task: tokio::task::JoinHandle<()>,
}

impl Drop for AccessibilityManager {
    fn drop(&mut self) {
        self._event_task.abort();
    }
}

impl AccessibilityManager {
    pub async fn new() -> Result<Self, Box<dyn Error + Send + Sync>> {
        let connection = AccessibilityConnection::new().await?;

        // TODO: these registrations cause ibus-extension-gtk3 to spin after suspend/wake.
        // Commented out until we find a fix that doesn't require on-demand tree traversal
        // (which is prohibitively slow when Firefox is focused — see memory note
        // atspi-tree-traversal-perf). With these commented out, cursor context is
        // silently unavailable; the event stream runs but never receives events.
        // connection.register_event::<StateChangedEvent>().await?;
        // connection.register_event::<ActivateEvent>().await?;

        let focused: Arc<Mutex<Option<ObjectRefOwned>>> = Arc::new(Mutex::new(None));
        let focused_clone = focused.clone();
        let mut event_stream = connection.event_stream();
        let (focus_tx, focus_receiver) = watch::channel(());

        let event_task = tokio::spawn(async move {
            while let Some(result) = event_stream.next().await {
                match result {
                    Ok(Event::Object(atspi::connection::common::events::ObjectEvents::StateChanged(ev))) => {
                        if ev.state == State::Focused && ev.enabled {
                            if let Ok(mut lock) = focused_clone.lock() {
                                *lock = Some(ev.item);
                            }
                            let _ = focus_tx.send(());
                        }
                    }
                    Ok(Event::Window(WindowEvents::Activate(_))) => {
                        // Window switched — stale detection in daemon.rs handles this
                    }
                    _ => {}
                }
            }
            eprintln!("[AT-SPI] event stream ended");
        });

        Ok(Self { connection, focused, focus_receiver, _event_task: event_task })
    }

    /// Returns information about the text cursor in the currently focused element.
    /// Uses the event-cached focused object — no tree traversal needed.
    /// Returns None when event registration is disabled (see TODO in new()).
    pub async fn get_cursor_info(&self) -> Result<Option<CursorInfo>, Box<dyn Error + Send + Sync>> {
        let focused_ref = match self.focused.lock().ok().and_then(|g| g.clone()) {
            Some(r) => r,
            None => return Ok(None),
        };

        let node = match focused_ref.as_accessible_proxy(self.connection.connection()).await {
            Ok(n) => n,
            Err(_) => return Ok(None),
        };

        let proxies = match node.proxies().await {
            Ok(p) => p,
            Err(_) => return Ok(None),
        };

        let text_proxy = match proxies.text().await {
            Ok(t) => t,
            Err(_) => return Ok(None),
        };

        let offset = match text_proxy.caret_offset().await {
            Ok(o) if o >= 0 => o,
            _ => return Ok(None),
        };

        let start = (offset - 100).max(0);
        let text_before = text_proxy.get_text(start, offset).await.unwrap_or_default();
        Ok(Some(CursorInfo { _offset: offset, text_before }))
    }

    /// Returns the text content of the currently focused element and its ancestors.
    /// Used by the `at-spi` debug command only — tree traversal is acceptable here.
    pub async fn get_focused_context(&self) -> Result<Vec<String>, Box<dyn Error + Send + Sync>> {
        let active_window_ref = self.find_active_window_ref().await?;

        if let Some(window_ref) = active_window_ref {
            if let Ok(window) = window_ref.as_accessible_proxy(self.connection.connection()).await {
                let focused_ref = self.find_focused_node_ref(&window, 0).await?;
                let mut context = Vec::new();

                if let Some(mut current_ref) = focused_ref {
                    for _ in 0..20 {
                        if let Ok(node) = current_ref.as_accessible_proxy(self.connection.connection()).await {
                            if let Ok(Some(text)) = self.get_text_if_supported(&node).await {
                                let trimmed = text.trim().replace('\u{fffc}', "");
                                if !trimmed.is_empty() {
                                    context.push(trimmed);
                                }
                            }

                            if let Ok(parent_ref) = node.parent().await {
                                if parent_ref.is_null() { break; }
                                current_ref = parent_ref;
                            } else {
                                break;
                            }
                        } else {
                            break;
                        }
                    }
                }
                return Ok(context);
            }
        }

        Ok(Vec::new())
    }

    async fn find_active_window_ref(&self) -> Result<Option<ObjectRefOwned>, Box<dyn Error + Send + Sync>> {
        let root = self.connection.root_accessible_on_registry().await?;
        let child_count = root.child_count().await.unwrap_or(0);

        for i in 0..child_count {
            if let Ok(app_ref) = root.get_child_at_index(i).await {
                if let Ok(app) = app_ref.as_accessible_proxy(self.connection.connection()).await {
                    let app_child_count = app.child_count().await.unwrap_or(0);
                    for j in 0..app_child_count {
                        if let Ok(window_ref) = app.get_child_at_index(j).await {
                            if let Ok(window) = window_ref.as_accessible_proxy(self.connection.connection()).await {
                                let states = window.get_state().await.unwrap_or_default();
                                if states.contains(State::Active) {
                                    return Ok(Some(ObjectRefOwned::try_from(&window)?));
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(None)
    }

    async fn find_focused_node_ref(&self, accessible: &AccessibleProxy<'_>, depth: u32) -> Result<Option<ObjectRefOwned>, Box<dyn Error + Send + Sync>> {
        if depth > 50 { return Ok(None); }

        let states = match accessible.get_state().await {
            Ok(s) => s,
            Err(_) => return Ok(None),
        };

        if states.contains(State::Focused) {
            return Ok(Some(ObjectRefOwned::try_from(accessible).map_err(|e| Box::new(e) as Box<dyn Error + Send + Sync>)?));
        }

        let child_count = accessible.child_count().await.unwrap_or(0);
        for i in 0..child_count {
            if let Ok(child_ref) = accessible.get_child_at_index(i).await {
                if let Ok(child) = child_ref.as_accessible_proxy(self.connection.connection()).await {
                    if let Ok(Some(found)) = Box::pin(self.find_focused_node_ref(&child, depth + 1)).await {
                        return Ok(Some(found));
                    }
                }
            }
        }

        Ok(None)
    }

    async fn get_text_if_supported(&self, accessible: &AccessibleProxy<'_>) -> Result<Option<String>, Box<dyn Error + Send + Sync>> {
        let proxies = accessible.proxies().await?;
        if let Ok(text_proxy) = proxies.text().await {
            let text = text_proxy.get_text(0, -1).await?;
            Ok(Some(text))
        } else {
            Ok(None)
        }
    }

    pub async fn get_visible_text(&self) -> Result<Vec<String>, Box<dyn Error + Send + Sync>> {
        let active_window_ref = self.find_active_window_ref().await?;
        let mut visible_texts = Vec::new();

        if let Some(window_ref) = active_window_ref {
            if let Ok(window) = window_ref.as_accessible_proxy(self.connection.connection()).await {
                let win_name = window.name().await.unwrap_or_default();
                eprintln!("[DEBUG] Found Active Window: '{}'", win_name);
                self.collect_visible_text_recursive(&window, &mut visible_texts, 0).await?;
            }
        }

        Ok(visible_texts)
    }

    async fn collect_visible_text_recursive(&self, accessible: &AccessibleProxy<'_>, texts: &mut Vec<String>, depth: u32) -> Result<(), Box<dyn Error + Send + Sync>> {
        if depth > 20 { return Ok(()); }

        let states = match accessible.get_state().await {
            Ok(s) => s,
            Err(_) => return Ok(()),
        };

        if !states.contains(State::Visible) && !states.contains(State::Showing) {
            return Ok(());
        }

        if let Ok(Some(text)) = self.get_text_if_supported(accessible).await {
            let trimmed = text.trim().replace('\u{fffc}', "");
            if !trimmed.is_empty() {
                texts.push(trimmed);
            }
        }

        let child_count = accessible.child_count().await.unwrap_or(0);
        for i in 0..child_count {
            if let Ok(child_ref) = accessible.get_child_at_index(i).await {
                if let Ok(child) = child_ref.as_accessible_proxy(self.connection.connection()).await {
                    Box::pin(self.collect_visible_text_recursive(&child, texts, depth + 1)).await?;
                }
            }
        }

        Ok(())
    }
}
