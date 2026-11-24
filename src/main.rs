#![allow(clippy::collapsible_if)]
mod app;
mod downloads;
mod icons;
mod images;
mod jobs;
mod library;
mod local;
mod mangadex;
mod net;
mod reader;
mod ui;

fn main() -> eframe::Result<()> {
    let opts = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("MangaReader")
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([720.0, 520.0]),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "MangaReader",
        opts,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}
