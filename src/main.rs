#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod canvas;
#[cfg(target_os = "linux")]
mod desktop;
mod file;
mod floating;
mod history;
mod icons;
mod settings;
mod shortcuts;
mod text;
mod titlebar;

/// Names the window for the desktop: Wayland's app id and X11's WM_CLASS.
const APP_ID: &str = "paint";

fn main() -> eframe::Result {
    #[cfg(target_os = "linux")]
    desktop::install();

    let options = eframe::NativeOptions {
        viewport: titlebar::configure(eframe::egui::ViewportBuilder::default())
            .with_title("Paint")
            .with_app_id(APP_ID)
            .with_inner_size([1280.0, 800.0])
            .with_icon(
                eframe::icon_data::from_png_bytes(include_bytes!("../assets/icons/256.png"))
                    .expect("bundled icon is a valid PNG"),
            ),
        ..Default::default()
    };
    eframe::run_native(
        "Paint",
        options,
        Box::new(|cc| {
            let mut app = app::PaintApp::new(cc);
            // Opening an image from the command line / "Open with".
            if let Some(path) = std::env::args_os().nth(1) {
                app.open_path(path.into());
            }
            Ok(Box::new(app))
        }),
    )
}
