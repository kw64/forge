//! MediaForge - interface graphique (egui). v0.3.0
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod batch;
mod edit;
mod i18n;
mod look;
mod online;
mod shell;
mod spec_ui;
mod tools;
mod util;

fn main() -> eframe::Result<()> {
    // Fichiers passés en argument (« Ouvrir avec », menu contextuel) : ouverts dans l'instance existante si elle tourne.
    let cwd = std::env::current_dir().unwrap_or_default();
    let files: Vec<std::path::PathBuf> = std::env::args_os()
        .skip(1)
        .map(std::path::PathBuf::from)
        .map(|p| if p.is_absolute() { p } else { cwd.join(p) })
        .collect();
    let listener = match ff_core::instance::acquire(47831, &files) {
        ff_core::instance::Acquired::Forwarded => return Ok(()),
        ff_core::instance::Acquired::Primary(l) => l,
    };

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("MediaForge")
            .with_inner_size([980.0, 640.0])
            .with_min_inner_size([640.0, 420.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        "MediaForge",
        options,
        Box::new(move |cc| {
            let mut a = app::App::new(cc, listener);
            a.add_files(files.into_iter().flat_map(util::expand_dropped).filter(|p| p.is_file()).collect());
            Ok(Box::new(a))
        }),
    )
}
