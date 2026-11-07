//! Aide locale : ouvre les pages du minisite `www/aide/` (livré à côté des exécutables) dans le navigateur. v0.16.0
#![allow(deprecated)] // Id::new : identifiant stable partagé entre applications
use egui;
use std::path::PathBuf;

/// Dossier `www/aide` : à côté de l'exécutable, ou dans un dossier parent (développement : target/release/…).
pub fn find_dir() -> Option<PathBuf> {
    let mut bases: Vec<PathBuf> = Vec::new();
    if let Ok(me) = std::env::current_exe() {
        let mut d = me.parent().map(|p| p.to_path_buf());
        for _ in 0..4 {
            if let Some(x) = d {
                bases.push(x.clone());
                d = x.parent().map(|p| p.to_path_buf());
            } else {
                break;
            }
        }
    }
    if let Ok(c) = std::env::current_dir() {
        bases.push(c);
    }
    bases.into_iter().map(|b| b.join("www").join("aide")).find(|d| d.join("index.html").exists())
}

/// URL `file://` avec les caractères spéciaux encodés.
fn file_url(p: &std::path::Path, anchor: &str) -> String {
    let s = p.to_string_lossy().replace('\\', "/");
    let mut out = String::from("file:///");
    out.push_str(s.trim_start_matches('/'));
    let enc: String = out.chars().flat_map(|c| match c {
        ' ' => "%20".chars().collect::<Vec<_>>(),
        '#' => "%23".chars().collect(),
        '%' => "%25".chars().collect(),
        _ => vec![c],
    }).collect();
    if anchor.is_empty() { enc } else { format!("{enc}#{anchor}") }
}

/// Ouvre `page` (ex. « mediaforge ») à l'ancre `anchor` dans le navigateur par défaut.
pub fn open(page: &str, anchor: &str) -> Result<(), String> {
    let dir = find_dir().ok_or("dossier www/aide introuvable (à placer à côté de l'exécutable)")?;
    let file = dir.join(format!("{page}.html"));
    if !file.exists() {
        return Err(format!("page d'aide absente : {}", file.display()));
    }
    let url = file_url(&file, anchor);
    #[cfg(windows)]
    {
        std::process::Command::new("rundll32").arg("url.dll,FileProtocolHandler").arg(&url).spawn().map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(&url).spawn().map_err(|e| e.to_string())?;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open").arg(&url).spawn().map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Petit bouton « ? » : ouvre l'aide à l'ancre donnée ; en cas d'échec, message dans une fenêtre.
pub fn button(ui: &mut egui::Ui, page: &str, anchor: &str, tip: &str) -> egui::Response {
    let r = ui.add(egui::Button::new(egui::RichText::new("?").strong()).small().corner_radius(10.0)).on_hover_text(tip);
    if r.clicked() {
        trigger(ui.ctx(), page, anchor);
    }
    r
}

/// Déclenche l'ouverture (raccourci F1, menus…) ; mémorise une éventuelle erreur à afficher.
pub fn trigger(ctx: &egui::Context, page: &str, anchor: &str) {
    if let Err(e) = open(page, anchor) {
        ctx.data_mut(|d| d.insert_temp(egui::Id::new("ff_help_err"), e));
    }
}

/// À appeler une fois par image : affiche l'erreur d'aide éventuelle.
pub fn show_error(ctx: &egui::Context) {
    let id = egui::Id::new("ff_help_err");
    let Some(e) = ctx.data(|d| d.get_temp::<String>(id)) else { return };
    let mut open = true;
    egui::Window::new("Aide").open(&mut open).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
        ui.label(&e);
        ui.label(egui::RichText::new("www/aide / help folder: place it next to the executable.").small().weak());
    });
    if !open {
        ctx.data_mut(|d| d.remove_temp::<String>(id));
    }
}

/// Rangée de liens d'aide (« ? libellé ») vers plusieurs ancres d'une même page.
pub fn links(ui: &mut egui::Ui, page: &str, items: &[(&str, &str)]) {
    ui.horizontal_wrapped(|ui| {
        ui.label(egui::RichText::new("?").strong());
        for (label, anchor) in items {
            if ui.link(*label).clicked() {
                trigger(ui.ctx(), page, anchor);
            }
        }
    });
}
