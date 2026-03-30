use gtk4::prelude::*;
use gtk4::{glib, Button, Window, Box as GtkBox, Orientation};
use std::sync::{Arc, Mutex};
use tokio::runtime::Runtime;
use zbus::Connection;
use crate::config::Config;
use crate::extension_proxy::ExtensionProxy;

pub fn record_shortcut(
    parent_window: &libadwaita::ApplicationWindow,
    config: Arc<Mutex<Config>>,
    runtime: Arc<Runtime>,
    on_complete: impl Fn(String) + 'static,
) {
    let window_clone = parent_window.clone();
    let _config_clone = config.clone();
    let runtime_clone = runtime.clone();
    let on_complete = Arc::new(on_complete);

    // Unregister current shortcut while recording
    let rt = runtime_clone.clone();
    rt.spawn(async move {
        let conn = Connection::session().await.ok();
        if let Some(c) = conn {
            if let Ok(proxy) = ExtensionProxy::new(&c).await {
                let _ = proxy.unregister_shortcut().await;
            }
        }
    });

    // Create recording modal
    let dialog = Window::builder()
        .title("Record Shortcut")
        .default_width(300)
        .default_height(150)
        .modal(true)
        .transient_for(&window_clone)
        .build();

    let vbox = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(12)
        .margin_top(24)
        .margin_bottom(24)
        .margin_start(24)
        .margin_end(24)
        .build();

    let label = gtk4::Label::new(Some("Press the key combination you want to use"));
    vbox.append(&label);

    let status_label = gtk4::Label::new(Some("Listening..."));
    vbox.append(&status_label);

    let cancel_button = Button::with_label("Cancel");
    vbox.append(&cancel_button);

    dialog.set_child(Some(&vbox));

    let key_controller = gtk4::EventControllerKey::new();
    let dialog_clone = dialog.clone();
    let config_key = config.clone();
    let runtime_key = runtime.clone();
    let on_complete_key = on_complete.clone();
    
    key_controller.connect_key_pressed(move |_controller, keyval, _keycode, state| {
        let modifiers = state & (gtk4::gdk::ModifierType::CONTROL_MASK | 
                                gtk4::gdk::ModifierType::ALT_MASK | 
                                gtk4::gdk::ModifierType::SHIFT_MASK | 
                                gtk4::gdk::ModifierType::SUPER_MASK);
        if modifiers.is_empty() && keyval.name().map(|n| n.starts_with("F")).unwrap_or(false) == false {
            return glib::Propagation::Proceed;
        }

        let mut accel = String::new();
        if state.contains(gtk4::gdk::ModifierType::CONTROL_MASK) { accel.push_str("<Control>"); }
        if state.contains(gtk4::gdk::ModifierType::ALT_MASK) { accel.push_str("<Alt>"); }
        if state.contains(gtk4::gdk::ModifierType::SHIFT_MASK) { accel.push_str("<Shift>"); }
        if state.contains(gtk4::gdk::ModifierType::SUPER_MASK) { accel.push_str("<Super>"); }

        if let Some(name) = keyval.name() {
            let name = match name.as_str() {
                "Control_L" | "Control_R" | "Alt_L" | "Alt_R" | "Shift_L" | "Shift_R" | "Super_L" | "Super_R" => return glib::Propagation::Proceed,
                n => n,
            };
            accel.push_str(name);
        }

        if !accel.is_empty() {
            let mut cfg = config_key.lock().unwrap();
            cfg.shortcut = accel.clone();
            cfg.save();

            let rt = runtime_key.clone();
            let accel_to_reg = accel.clone();
            rt.spawn(async move {
                let conn = Connection::session().await.ok();
                if let Some(c) = conn {
                    if let Ok(proxy) = ExtensionProxy::new(&c).await {
                        let _ = proxy.register_shortcut(&accel_to_reg).await;
                    }
                }
            });

            on_complete_key(accel);
            dialog_clone.close();
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });

    dialog.add_controller(key_controller);
    
    let dialog_cancel = dialog.clone();
    let runtime_cancel = runtime.clone();
    let config_cancel = config.clone();
    cancel_button.connect_clicked(move |_| {
        let prev_accel = config_cancel.lock().unwrap().shortcut.clone();
        let rt = runtime_cancel.clone();
        rt.spawn(async move {
            if !prev_accel.is_empty() {
                let conn = Connection::session().await.ok();
                if let Some(c) = conn {
                    if let Ok(proxy) = ExtensionProxy::new(&c).await {
                        let _ = proxy.register_shortcut(&prev_accel).await;
                    }
                }
            }
        });
        dialog_cancel.close();
    });

    dialog.present();
}
