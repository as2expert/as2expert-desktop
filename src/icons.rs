//! Bundled Silk icons (famfamfam, CC BY 2.5), embedded at compile time and
//! rendered through egui's image loaders. Each icon is a 16×16 RGBA PNG.

use eframe::egui::{self, ImageSource};

/// The named icons used across the UI.
#[derive(Clone, Copy)]
pub enum Icon {
    Refresh,
    Compose,
    Send,
    Delete,
    Read,
    Unread,
    Message,
    Save,
    Search,
    Add,
    Folder,
    Inbox,
    Lock,
    Ok,
    Warning,
    In,
    Out,
    Station,
    Partner,
    Certificate,
    Key,
    Attach,
    Logo,
}

/// Resolve an [`Icon`] to its embedded image source.
pub fn source(icon: Icon) -> ImageSource<'static> {
    match icon {
        Icon::Refresh => egui::include_image!("../assets/icons/refresh.png"),
        Icon::Compose => egui::include_image!("../assets/icons/compose.png"),
        Icon::Send => egui::include_image!("../assets/icons/send.png"),
        Icon::Delete => egui::include_image!("../assets/icons/delete.png"),
        Icon::Read => egui::include_image!("../assets/icons/read.png"),
        Icon::Unread => egui::include_image!("../assets/icons/unread.png"),
        Icon::Message => egui::include_image!("../assets/icons/message.png"),
        Icon::Save => egui::include_image!("../assets/icons/save.png"),
        Icon::Search => egui::include_image!("../assets/icons/search.png"),
        Icon::Add => egui::include_image!("../assets/icons/add.png"),
        Icon::Folder => egui::include_image!("../assets/icons/folder.png"),
        Icon::Inbox => egui::include_image!("../assets/icons/inbox.png"),
        Icon::Lock => egui::include_image!("../assets/icons/lock.png"),
        Icon::Ok => egui::include_image!("../assets/icons/ok.png"),
        Icon::Warning => egui::include_image!("../assets/icons/warning.png"),
        Icon::In => egui::include_image!("../assets/icons/in.png"),
        Icon::Out => egui::include_image!("../assets/icons/out.png"),
        Icon::Station => egui::include_image!("../assets/icons/station.png"),
        Icon::Partner => egui::include_image!("../assets/icons/partner.png"),
        Icon::Certificate => egui::include_image!("../assets/icons/certificate.png"),
        Icon::Key => egui::include_image!("../assets/icons/key.png"),
        Icon::Attach => egui::include_image!("../assets/icons/attach.png"),
        Icon::Logo => egui::include_image!("../assets/icons/logo.png"),
    }
}

/// An [`egui::Image`] for `icon`, sized to `px` square.
pub fn image(icon: Icon, px: f32) -> egui::Image<'static> {
    egui::Image::new(source(icon)).fit_to_exact_size(egui::vec2(px, px))
}

/// Draw an icon inline at `px` square.
pub fn show(ui: &mut egui::Ui, icon: Icon, px: f32) -> egui::Response {
    ui.add(image(icon, px))
}

/// A flat icon-only button with a tooltip.
pub fn tool_button(ui: &mut egui::Ui, icon: Icon, px: f32, tip: &str) -> egui::Response {
    ui.add(egui::ImageButton::new(image(icon, px)).frame(false))
        .on_hover_text(tip)
}

/// A toolbar button showing an icon and a label.
pub fn labeled_button(ui: &mut egui::Ui, icon: Icon, px: f32, text: &str) -> egui::Response {
    ui.add(egui::Button::image_and_text(image(icon, px), text))
}
