// use zbus::proxy;

#[zbus::proxy(
    interface = "com.timcharper.dictation.Extension",
    default_service = "com.timcharper.dictation.Extension",
    default_path = "/com/timcharper/dictation/Extension"
)]
pub trait Extension {
    async fn update(&self, icon_name: &str, menu_items: Vec<(String, String)>, state: &str, color: &str) -> zbus::Result<()>;
    async fn raise_app(&self) -> zbus::Result<()>;
    async fn get_clipboard(&self) -> zbus::Result<String>;
    async fn set_clipboard(&self, text: &str) -> zbus::Result<()>;
    async fn type_string(&self, text: &str) -> zbus::Result<()>;
    async fn register_shortcut(&self, shortcut: &str) -> zbus::Result<()>;
    async fn unregister_shortcut(&self) -> zbus::Result<()>;
    async fn get_focused_window_class(&self) -> zbus::Result<String>;
    async fn get_focused_window_pid(&self) -> zbus::Result<u32>;

    #[zbus(signal)]
    async fn menu_item_selected(&self, id: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn shortcut_pressed(&self) -> zbus::Result<()>;
}
