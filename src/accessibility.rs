use atspi::proxy::accessible::AccessibleProxy;
use atspi::AccessibilityConnection;
use atspi::proxy::proxy_ext::ProxyExt;
use atspi::proxy::accessible::ObjectRefExt;
use atspi_common::{State, ObjectRefOwned};
use std::error::Error;

pub struct AccessibilityManager {
    connection: AccessibilityConnection,
}

impl AccessibilityManager {
    pub async fn new() -> Result<Self, Box<dyn Error + Send + Sync>> {
        let connection = AccessibilityConnection::new().await?;
        Ok(Self { connection })
    }

    /// Returns the text content of the currently focused element and its ancestors.
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
        let child_count = root.child_count().await?;

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

        let states = accessible.get_state().await?;
        if states.contains(State::Focused) {
            return Ok(Some(ObjectRefOwned::try_from(accessible)?));
        }

        let child_count = accessible.child_count().await?;
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
