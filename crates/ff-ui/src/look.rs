//! Palette courante (thème actif) accessible depuis tous les écrans. v0.5.0
use eframe::egui::Color32;
use ff_kit::theme::{palette, Palette, Style};
use std::sync::RwLock;

static CUR: RwLock<Option<Palette>> = RwLock::new(None);

pub fn set(p: Palette) {
    if let Ok(mut g) = CUR.write() {
        *g = Some(p);
    }
}

pub fn pal() -> Palette {
    CUR.read().ok().and_then(|g| *g).unwrap_or_else(|| palette(Style::Colorful, true))
}

pub fn accent() -> Color32 {
    pal().accent
}
pub fn green() -> Color32 {
    pal().ok
}
pub fn red() -> Color32 {
    pal().err
}
