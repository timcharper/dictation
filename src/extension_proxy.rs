// use zbus::proxy;

#[zbus::proxy(
    interface = "org.gnome.dictation.Extension",
    default_service = "org.gnome.dictation.Extension",
    default_path = "/org/gnome/dictation/Extension"
)]
pub trait Extension {
    async fn update(&self, icon_name: &str, menu_items: Vec<(&str, &str)>) -> zbus::Result<()>;
    async fn raise_app(&self) -> zbus::Result<()>;
    async fn get_clipboard(&self) -> zbus::Result<String>;
    async fn set_clipboard(&self, text: &str) -> zbus::Result<()>;
    async fn type_string(&self, text: &str) -> zbus::Result<()>;
    async fn register_shortcut(&self, shortcut: &str) -> zbus::Result<()>;
    async fn unregister_shortcut(&self) -> zbus::Result<()>;
    async fn get_volume(&self) -> zbus::Result<f64>;
    async fn set_volume(&self, volume: f64) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn menu_item_selected(&self, id: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn shortcut_pressed(&self) -> zbus::Result<()>;
}
