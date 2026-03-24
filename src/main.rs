#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod domain;
mod export;
mod platform;

use anyhow::{Context, Result};
use app::EventXmlExporterApp;
use eframe::{NativeOptions, egui, egui::IconData};
use image::ImageReader;
use std::io::Cursor;

fn main() -> eframe::Result<()> {
    let viewport = egui::ViewportBuilder::default()
        .with_title("Olay XML Dışa Aktarıcı")
        .with_inner_size([1360.0, 860.0])
        .with_min_inner_size([1180.0, 760.0]);

    let native_options = NativeOptions {
        viewport: match load_app_icon() {
            Ok(icon) => viewport.with_icon(icon),
            Err(_) => viewport,
        },
        ..Default::default()
    };

    eframe::run_native(
        "Olay XML Dışa Aktarıcı",
        native_options,
        Box::new(|cc| Ok(Box::new(EventXmlExporterApp::new(cc)))),
    )
}

fn load_app_icon() -> Result<IconData> {
    let icon_bytes = include_bytes!("../assets/app.ico");
    let image = ImageReader::new(Cursor::new(icon_bytes))
        .with_guessed_format()
        .context("Uygulama ikonu formatı algılanamadı")?
        .decode()
        .context("Uygulama ikonu çözümlenemedi")?
        .into_rgba8();

    let (width, height) = image.dimensions();

    Ok(IconData {
        rgba: image.into_raw(),
        width,
        height,
    })
}
