// AS2Expert Desktop — a native, cross-platform client for AS2/EDI messaging.
// Hides the console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod config;
mod remote;

use eframe::egui;

fn native_options() -> eframe::NativeOptions {
    eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("AS2Expert Desktop")
            .with_inner_size([1120.0, 720.0])
            .with_min_inner_size([820.0, 520.0]),
        ..Default::default()
    }
}

fn run() -> eframe::Result<()> {
    eframe::run_native(
        "AS2Expert Desktop",
        native_options(),
        Box::new(|cc| Ok(Box::new(app::App::new(cc)) as Box<dyn eframe::App>)),
    )
}

fn main() -> eframe::Result<()> {
    // Remote desktop sessions (RDP, most VNC/VDI) expose only OpenGL 1.1, so the
    // GPU-backed context fails. Detect that up front and switch to a software
    // renderer; local sessions keep hardware acceleration.
    let force_sw = std::env::var_os("AS2EXPERT_SOFTWARE_GL").is_some();
    if force_sw || remote::is_remote_session() {
        remote::enable_software_gl();
    }

    match run() {
        Ok(()) => Ok(()),
        Err(err) => {
            // Safety net: if the GPU context still failed to initialize (an
            // unusual remote setup, or a broken driver), retry once in software.
            eprintln!("Renderer init failed ({err}); retrying with software OpenGL…");
            remote::enable_software_gl();
            run()
        }
    }
}
