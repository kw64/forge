//! MediaForge Player - lecteur universel audio / vidéo / images (moteur libmpv). v0.4.0
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod derush;
mod doc;
mod i18n;
mod mesh;
mod playlist;
mod shell;
mod store;
mod video;

/// Port local de l'instance unique du lecteur (le convertisseur utilise 47831).
const PORT: u16 = 47832;

/// Démarrage réduit demandé (`--minimized`) : appliqué à la première image.
pub static START_MIN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn main() -> eframe::Result<()> {
    let cwd = std::env::current_dir().unwrap_or_default();
    // `--minimized` : démarrage réduit (lancement avec Windows) ; les options ne sont pas des fichiers.
    let minimized = std::env::args_os().any(|a| a == "--minimized");
    START_MIN.store(minimized, std::sync::atomic::Ordering::Relaxed);
    let files: Vec<std::path::PathBuf> = std::env::args_os()
        .skip(1)
        .filter(|a| !a.to_string_lossy().starts_with("--"))
        .map(std::path::PathBuf::from)
        .map(|p| if p.is_absolute() || p.to_string_lossy().starts_with("http") { p } else { cwd.join(p) })
        .collect();
    let listener = match ff_core::instance::acquire(PORT, &files) {
        ff_core::instance::Acquired::Forwarded => return Ok(()),
        ff_core::instance::Acquired::Primary(l) => l,
    };
    let settings = store::load_settings();
    if let Some(d) = &settings.mpv_dir {
        // Dossier de libmpv choisi par l'utilisateur (lu par ff-mpv).
        std::env::set_var("MEDIAFORGE_MPV_DIR", d);
    }
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("MediaForge Player")
            .with_inner_size([960.0, 600.0])
            .with_min_inner_size([360.0, 240.0])
            .with_drag_and_drop(true),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "MediaForge Player",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, listener, settings, files)))),
    )
}
