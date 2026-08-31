// AS2Expert Desktop — a native, cross-platform client for AS2/EDI messaging.
// Hides the console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod config;

use eframe::egui;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("AS2Expert Desktop")
            .with_inner_size([1120.0, 720.0])
            .with_min_inner_size([820.0, 520.0]),
        ..Default::default()
    };
    eframe::run_native(
        "AS2Expert Desktop",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}
