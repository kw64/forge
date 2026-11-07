//! Mini-éditeur : découpe (début/fin) et recadrage appliqués pendant la conversion. v0.3.0
use crate::app::{App, State};
#[allow(unused_imports)]
use crate::look::{accent, green, red};
use crate::i18n::{tr, trf};
use crate::util::fmt_secs;
use eframe::egui::{self, Color32, RichText};
use ff_core::{Crop, Edits, JobKind, Lang};

/// Brouillon d'édition d'un élément de la liste.
pub struct EditDraft {
    pub uid: u64,
    pub name: String,
    pub start: String,
    pub end: String,
    pub cw: String,
    pub ch: String,
    pub cx: String,
    pub cy: String,
    pub info: String,
    pub err: String,
    pub audio_only: bool,
}

/// Convertit « 90 », « 1:30 », « 1:02:03 » ou « 1:30.5 » en secondes ; chaîne vide = `Ok(None)`.
pub fn parse_time(s: &str) -> Result<Option<f64>, ()> {
    let s = s.trim().replace(',', ".");
    if s.is_empty() {
        return Ok(None);
    }
    let mut total = 0.0;
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() > 3 {
        return Err(());
    }
    for p in &parts {
        let v: f64 = p.trim().parse().map_err(|_| ())?;
        if v < 0.0 || !v.is_finite() {
            return Err(());
        }
        total = total * 60.0 + v;
    }
    Ok(Some(total))
}

fn parse_px(s: &str) -> Result<Option<u32>, ()> {
    let s = s.trim();
    if s.is_empty() { Ok(None) } else { s.parse().map(Some).map_err(|_| ()) }
}

/// Résumé affiché sous le nom du fichier.
pub fn summary(l: Lang, e: &Edits) -> Option<String> {
    if e.is_empty() {
        return None;
    }
    let mut parts = Vec::new();
    match (e.trim_start, e.trim_end) {
        (Some(s), Some(t)) => parts.push(format!("{} {} → {}", tr(l, "Découpe :"), fmt_secs(s), fmt_secs(t))),
        (Some(s), None) => parts.push(format!("{} {}", tr(l, "Début :"), fmt_secs(s))),
        (None, Some(t)) => parts.push(format!("{} {}", tr(l, "Fin :"), fmt_secs(t))),
        _ => {}
    }
    if let Some(c) = e.crop {
        parts.push(format!("{} {}×{} (+{},+{})", tr(l, "Recadrage :"), c.w, c.h, c.x, c.y));
    }
    Some(format!("✂ {}", parts.join("  ·  ")))
}

fn fmt_opt(v: Option<f64>) -> String {
    v.map(|s| {
        let t = s.max(0.0);
        let (m, sec) = ((t / 60.0).floor() as u64, t % 60.0);
        if sec.fract() == 0.0 { format!("{m}:{:02}", sec as u64) } else { format!("{m}:{sec:05.2}") }
    })
    .unwrap_or_default()
}

impl App {
    pub(crate) fn open_edit(&mut self, idx: usize) {
        let l = self.settings.lang;
        let Some(it) = self.items.get(idx) else { return };
        if it.kind != JobKind::Convert || it.state != State::Idle {
            return;
        }
        let (mut info, mut audio_only) = (String::new(), it.profile.is_audio_only());
        if let Some(b) = &self.bins {
            if let Ok(m) = ff_probe::probe(&b.ffprobe, &it.input) {
                audio_only = audio_only || !m.has_video;
                let mut parts = Vec::new();
                if let Some((w, h)) = m.size {
                    parts.push(format!("{w}×{h}"));
                }
                if let Some(d) = m.duration_secs.filter(|d| *d > 0.0) {
                    parts.push(format!("{} {}", tr(l, "durée"), fmt_secs(d)));
                }
                info = parts.join(" · ");
            }
        }
        let e = &it.edits;
        let c = e.crop;
        self.edit = Some(EditDraft {
            uid: it.uid,
            name: it.display_name(),
            start: fmt_opt(e.trim_start),
            end: fmt_opt(e.trim_end),
            cw: c.map(|c| c.w.to_string()).unwrap_or_default(),
            ch: c.map(|c| c.h.to_string()).unwrap_or_default(),
            cx: c.map(|c| c.x.to_string()).unwrap_or_default(),
            cy: c.map(|c| c.y.to_string()).unwrap_or_default(),
            info,
            err: String::new(),
            audio_only,
        });
    }

    pub(crate) fn edit_window(&mut self, ctx: &egui::Context) {
        let l = self.settings.lang;
        let mut open = true;
        let (mut apply, mut reset) = (false, false);
        if let Some(d) = self.edit.as_mut() {
            egui::Window::new(tr(l, "Retouches")).open(&mut open).resizable(false).show(ctx, |ui| {
                ff_kit::help::links(ui, "mediaforge", &[("Découpe", "decoupe"), ("Recadrage", "recadrage")]);
                ui.label(RichText::new(&d.name).strong());
                if !d.info.is_empty() {
                    ui.label(RichText::new(format!("{} {}", tr(l, "Source :"), d.info)).small().weak());
                }
                ui.separator();
                ui.label(RichText::new(tr(l, "Découpe (secondes ou mm:ss, vide = tout le fichier)")).strong());
                egui::Grid::new("edit_trim").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                    ui.label(tr(l, "Début :"));
                    ui.add(egui::TextEdit::singleline(&mut d.start).desired_width(120.0).hint_text("0:00"));
                    ui.end_row();
                    ui.label(tr(l, "Fin :"));
                    ui.add(egui::TextEdit::singleline(&mut d.end).desired_width(120.0).hint_text("1:30"));
                    ui.end_row();
                });
                ui.add_enabled_ui(!d.audio_only, |ui| {
                    ui.add_space(6.0);
                    ui.label(RichText::new(tr(l, "Recadrage (pixels, vide = aucun)")).strong());
                    egui::Grid::new("edit_crop").num_columns(4).spacing([8.0, 6.0]).show(ui, |ui| {
                        ui.label(tr(l, "Largeur :"));
                        ui.add(egui::TextEdit::singleline(&mut d.cw).desired_width(70.0));
                        ui.label(tr(l, "Hauteur :"));
                        ui.add(egui::TextEdit::singleline(&mut d.ch).desired_width(70.0));
                        ui.end_row();
                        ui.label("X :");
                        ui.add(egui::TextEdit::singleline(&mut d.cx).desired_width(70.0).hint_text("0"));
                        ui.label("Y :");
                        ui.add(egui::TextEdit::singleline(&mut d.cy).desired_width(70.0).hint_text("0"));
                        ui.end_row();
                    });
                });
                if d.audio_only {
                    ui.label(RichText::new(tr(l, "Recadrage indisponible pour de l'audio.")).small().weak());
                }
                ui.label(RichText::new(tr(l, "Avec un profil de capture d'image, le début choisit l'image extraite.")).small().weak());
                if !d.err.is_empty() {
                    ui.colored_label(Color32::from_rgb(0xE0, 0x31, 0x31), &d.err);
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.add(egui::Button::new(RichText::new(tr(l, "Appliquer")).color(Color32::WHITE)).fill(Color32::from_rgb(0xE8, 0x59, 0x0C))).clicked() {
                        apply = true;
                    }
                    if ui.button(tr(l, "Réinitialiser")).clicked() {
                        reset = true;
                    }
                });
            });
        }
        if reset {
            if let Some(d) = self.edit.as_mut() {
                (d.start, d.end, d.cw, d.ch, d.cx, d.cy) = Default::default();
                d.err.clear();
            }
        }
        if apply {
            self.apply_edit();
        } else if !open {
            self.edit = None;
        }
    }

    fn apply_edit(&mut self) {
        let l = self.settings.lang;
        let Some(d) = self.edit.as_mut() else { return };
        let res: Result<Edits, String> = (|| {
            let start = parse_time(&d.start).map_err(|_| tr(l, "Début invalide (ex. 90, 1:30).").to_string())?;
            let end = parse_time(&d.end).map_err(|_| tr(l, "Fin invalide (ex. 90, 1:30).").to_string())?;
            if let (Some(s), Some(e)) = (start, end) {
                if e <= s {
                    return Err(tr(l, "La fin doit être après le début.").into());
                }
            }
            let bad = || tr(l, "Recadrage invalide : nombres entiers positifs.").to_string();
            let (w, h) = (parse_px(&d.cw).map_err(|_| bad())?, parse_px(&d.ch).map_err(|_| bad())?);
            let (x, y) = (parse_px(&d.cx).map_err(|_| bad())?, parse_px(&d.cy).map_err(|_| bad())?);
            let crop = match (w, h) {
                (None, None) if x.is_none() && y.is_none() => None,
                (Some(w), Some(h)) if w >= 2 && h >= 2 => Some(Crop { w, h, x: x.unwrap_or(0), y: y.unwrap_or(0) }),
                _ => return Err(tr(l, "Recadrage : largeur et hauteur (au moins 2) sont requises.").into()),
            };
            Ok(Edits { trim_start: start, trim_end: end, crop })
        })();
        match res {
            Ok(e) => {
                let uid = d.uid;
                if let Some(it) = self.items.iter_mut().find(|i| i.uid == uid) {
                    it.edits = e;
                }
                self.edit = None;
            }
            Err(msg) => d.err = msg,
        }
        let _ = trf; // (réservé aux messages paramétrés)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn times() {
        assert_eq!(parse_time("90"), Ok(Some(90.0)));
        assert_eq!(parse_time("1:30"), Ok(Some(90.0)));
        assert_eq!(parse_time("1:02:03"), Ok(Some(3723.0)));
        assert_eq!(parse_time("1:30,5"), Ok(Some(90.5)));
        assert_eq!(parse_time(" "), Ok(None));
        assert!(parse_time("abc").is_err());
        assert!(parse_time("-5").is_err());
        assert!(parse_time("1:2:3:4").is_err());
    }
}
