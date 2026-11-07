//! Images par lots : redimensionnement et conversion avec politique d'écrasement (moteur : ff-image). v0.8.0
use crate::i18n::{tr, trf};
use crate::look::{accent, red};
use eframe::egui::{self, Color32, RichText};
use ff_core::Lang;
use ff_image::{Options, OutFormat, Overwrite, Resize, Unit};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

pub const IMG_EXT: &[&str] = &["jpg", "jpeg", "png", "webp", "bmp", "tif", "tiff", "gif"];

/// Travail en cours (partagé avec les threads de traitement).
pub struct Job {
    total: usize,
    done: AtomicUsize,
    ok: AtomicUsize,
    skipped: AtomicUsize,
    errors: Mutex<Vec<String>>,
    finished: AtomicBool,
    cancel: AtomicBool,
}

pub struct ImgState {
    pub files: Vec<PathBuf>,
    resize_on: bool,
    mode: u8,
    w: f64,
    h: f64,
    keep: bool,
    unit: u8,
    dpi: f64,
    long: f64,
    short: f64,
    mpx: f64,
    pw: f64,
    ph: f64,
    no_upscale: bool,
    fmt: usize,
    quality: u8,
    ow: u8,
    backup: bool,
    out_dir: Option<PathBuf>,
    suffix: String,
    confirm: bool,
    job: Option<Arc<Job>>,
    msg: String,
}

impl Default for ImgState {
    fn default() -> Self {
        ImgState {
            files: Vec::new(),
            resize_on: true,
            mode: 0,
            w: 1920.0,
            h: 0.0,
            keep: true,
            unit: 0,
            dpi: 300.0,
            long: 1920.0,
            short: 1080.0,
            mpx: 2.0,
            pw: 50.0,
            ph: 50.0,
            no_upscale: true,
            fmt: 0,
            quality: 90,
            ow: 0,
            backup: false,
            out_dir: None,
            suffix: String::new(),
            confirm: false,
            job: None,
            msg: String::new(),
        }
    }
}

impl ImgState {
    fn options(&self) -> Options {
        let resize = if !self.resize_on {
            Resize::None
        } else {
            match self.mode {
                0 => Resize::Sides { w: self.w, h: self.h, keep_ratio: self.keep },
                1 => Resize::Long(self.long),
                2 => Resize::Short(self.short),
                3 => Resize::Megapixels(self.mpx),
                _ => Resize::Percent { w: self.pw, h: if self.keep { self.pw } else { self.ph } },
            }
        };
        Options {
            resize,
            unit: [Unit::Px, Unit::Cm, Unit::Inch][self.unit.min(2) as usize],
            dpi: self.dpi.max(1.0),
            no_upscale: self.no_upscale,
            format: OutFormat::ALL[self.fmt.min(OutFormat::ALL.len() - 1)],
            quality: self.quality,
            overwrite: [Overwrite::Rename, Overwrite::Replace, Overwrite::Skip][self.ow.min(2) as usize],
            backup: self.backup,
            out_dir: self.out_dir.clone(),
            suffix: self.suffix.clone(),
        }
    }

    fn start(&mut self) {
        let o = Arc::new(self.options());
        let files = Arc::new(std::mem::take(&mut self.files));
        let job = Arc::new(Job { total: files.len(), done: AtomicUsize::new(0), ok: AtomicUsize::new(0), skipped: AtomicUsize::new(0), errors: Mutex::new(Vec::new()), finished: AtomicBool::new(false), cancel: AtomicBool::new(false) });
        let next = Arc::new(AtomicUsize::new(0));
        let n = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2).clamp(1, 8).min(files.len().max(1));
        let alive = Arc::new(AtomicUsize::new(n));
        for _ in 0..n {
            let (job, files, o, next, alive) = (job.clone(), files.clone(), o.clone(), next.clone(), alive.clone());
            std::thread::spawn(move || {
                loop {
                    let i = next.fetch_add(1, Ordering::SeqCst);
                    if i >= files.len() || job.cancel.load(Ordering::SeqCst) {
                        break;
                    }
                    match ff_image::process(&files[i], &o) {
                        Ok(ff_image::Outcome::Written(_)) => {
                            job.ok.fetch_add(1, Ordering::SeqCst);
                        }
                        Ok(ff_image::Outcome::Skipped) => {
                            job.skipped.fetch_add(1, Ordering::SeqCst);
                        }
                        Err(e) => {
                            let name = files[i].file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                            ff_profiles::log(&format!("ERREUR image {name} : {e}"));
                            if let Ok(mut v) = job.errors.lock() {
                                v.push(format!("{name} : {e}"));
                            }
                        }
                    }
                    job.done.fetch_add(1, Ordering::SeqCst);
                }
                if alive.fetch_sub(1, Ordering::SeqCst) == 1 {
                    job.finished.store(true, Ordering::SeqCst);
                }
            });
        }
        self.job = Some(job);
        self.confirm = false;
    }
}

fn num(ui: &mut egui::Ui, v: &mut f64, max: f64, dec: usize) {
    ui.add(egui::DragValue::new(v).range(0.0..=max).max_decimals(dec).speed(if dec > 0 { 0.1 } else { 1.0 }));
}

/// Contenu de l'onglet « Images par lots ».
pub fn images_tab(st: &mut ImgState, ui: &mut egui::Ui, l: Lang) {
    ui.label(tr(l, "Redimensionne et convertit des images, sans passer par ffmpeg."));
    // Liste des fichiers (ajout de fichiers ou de dossiers).
    ui.horizontal(|ui| {
        if ui.button(tr(l, "+ Ajouter des fichiers")).clicked() {
            if let Some(f) = rfd::FileDialog::new().add_filter(tr(l, "Images"), IMG_EXT).pick_files() {
                st.files.extend(f);
            }
        }
        if ui.button(tr(l, "+ Ajouter un dossier")).clicked() {
            if let Some(d) = rfd::FileDialog::new().pick_folder() {
                if let Ok(rd) = std::fs::read_dir(d) {
                    let mut v: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_file() && p.extension().map(|e| IMG_EXT.contains(&e.to_string_lossy().to_ascii_lowercase().as_str())).unwrap_or(false)).collect();
                    v.sort();
                    st.files.extend(v);
                }
            }
        }
        if ui.add_enabled(!st.files.is_empty(), egui::Button::new(tr(l, "Vider"))).clicked() {
            st.files.clear();
        }
        ui.label(RichText::new(trf(l, "{n} image(s)", &[("{n}", st.files.len().to_string())])).weak());
    });
    egui::ScrollArea::vertical().id_salt("imgfiles").max_height(90.0).show(ui, |ui| {
        let mut del = None;
        for (i, f) in st.files.iter().enumerate().take(500) {
            ui.horizontal(|ui| {
                if ui.small_button("x").clicked() {
                    del = Some(i);
                }
                ui.label(RichText::new(f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()).small());
            });
        }
        if let Some(i) = del {
            st.files.remove(i);
        }
    });
    ui.separator();

    // Redimensionnement
    ui.checkbox(&mut st.resize_on, RichText::new(tr(l, "Redimensionner")).strong());
    ui.add_enabled_ui(st.resize_on, |ui| {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.radio_value(&mut st.mode, 0, tr(l, "Mettre 1 ou 2 côtés à :"));
                ui.horizontal(|ui| {
                    ui.add_space(20.0);
                    ui.label(tr(l, "Larg. :"));
                    num(ui, &mut st.w, 100000.0, 2);
                    ui.label(tr(l, "Haut. :"));
                    num(ui, &mut st.h, 100000.0, 2);
                });
                ui.radio_value(&mut st.mode, 1, tr(l, "Long côté à :"));
                ui.horizontal(|ui| {
                    ui.add_space(20.0);
                    num(ui, &mut st.long, 100000.0, 2);
                });
                ui.radio_value(&mut st.mode, 2, tr(l, "Petit côté à :"));
                ui.horizontal(|ui| {
                    ui.add_space(20.0);
                    num(ui, &mut st.short, 100000.0, 2);
                });
                ui.radio_value(&mut st.mode, 3, tr(l, "Régler taille image à :"));
                ui.horizontal(|ui| {
                    ui.add_space(20.0);
                    num(ui, &mut st.mpx, 1000.0, 2);
                    ui.label("MegaPixel");
                });
                ui.radio_value(&mut st.mode, 4, tr(l, "Nouvelle taille en % de l'original :"));
                ui.horizontal(|ui| {
                    ui.add_space(20.0);
                    ui.label(tr(l, "Larg. :"));
                    num(ui, &mut st.pw, 1000.0, 1);
                    ui.label("%");
                    if !st.keep {
                        ui.label(tr(l, "Haut. :"));
                        num(ui, &mut st.ph, 1000.0, 1);
                        ui.label("%");
                    }
                });
            });
            ui.separator();
            ui.vertical(|ui| {
                ui.radio_value(&mut st.unit, 0, "pixels");
                ui.radio_value(&mut st.unit, 1, "cm");
                ui.radio_value(&mut st.unit, 2, tr(l, "pouces"));
                if st.unit != 0 {
                    ui.horizontal(|ui| {
                        ui.label("dpi");
                        num(ui, &mut st.dpi, 2400.0, 0);
                    });
                }
            });
        });
        ui.horizontal(|ui| {
            ui.checkbox(&mut st.keep, tr(l, "Conserver les proportions"));
            ui.checkbox(&mut st.no_upscale, tr(l, "Ne jamais agrandir"));
        });
    });
    ui.separator();

    // Format de sortie
    ui.horizontal(|ui| {
        ui.label(tr(l, "Format :"));
        egui::ComboBox::from_id_salt("imgfmt").selected_text(tr(l, OutFormat::ALL[st.fmt].label())).show_ui(ui, |ui| {
            for (i, f) in OutFormat::ALL.iter().enumerate() {
                ui.selectable_value(&mut st.fmt, i, tr(l, f.label()));
            }
        });
        if matches!(OutFormat::ALL[st.fmt], OutFormat::Jpeg | OutFormat::Same) {
            ui.label(tr(l, "Qualité JPEG :"));
            ui.add(egui::Slider::new(&mut st.quality, 40..=100));
        }
    });
    // Sortie et écrasement
    ui.horizontal(|ui| {
        ui.label(tr(l, "Dossier de sortie :"));
        if ui.button(tr(l, "Choisir…")).clicked() {
            if let Some(d) = rfd::FileDialog::new().pick_folder() {
                st.out_dir = Some(d);
            }
        }
        match &st.out_dir {
            Some(d) => {
                ui.label(RichText::new(d.display().to_string()).small());
                if ui.small_button("x").clicked() {
                    st.out_dir = None;
                }
            }
            None => {
                ui.label(RichText::new(tr(l, "dossier de la source")).weak());
            }
        }
    });
    ui.horizontal(|ui| {
        ui.label(tr(l, "Suffixe du nom :"));
        ui.add(egui::TextEdit::singleline(&mut st.suffix).desired_width(100.0).hint_text("_web"));
    });
    ui.horizontal(|ui| {
        ui.label(tr(l, "Si le fichier existe :"));
        ui.radio_value(&mut st.ow, 0, tr(l, "Ne pas écraser (renommer)"));
        ui.radio_value(&mut st.ow, 1, tr(l, "Écraser"));
        ui.radio_value(&mut st.ow, 2, tr(l, "Ignorer"));
    });
    if st.ow == 1 {
        ui.checkbox(&mut st.backup, tr(l, "Conserver l'ancien fichier en .bak"));
    }

    // Aperçu : combien de fichiers seraient remplacés.
    let o = st.options();
    let clash = if st.files.len() <= 2000 { st.files.iter().filter(|f| ff_image::planned_output(f, &o).exists()).count() } else { 0 };
    if clash > 0 {
        let txt = match st.ow {
            1 => trf(l, "{n} fichier(s) existant(s) seront remplacés.", &[("{n}", clash.to_string())]),
            2 => trf(l, "{n} fichier(s) existant(s) seront ignorés.", &[("{n}", clash.to_string())]),
            _ => trf(l, "{n} fichier(s) existant(s) : les nouveaux seront renommés (1).", &[("{n}", clash.to_string())]),
        };
        ui.colored_label(if st.ow == 1 { red() } else { ui.visuals().weak_text_color() }, txt);
    }
    let need_confirm = st.ow == 1 && clash > 0;
    if need_confirm {
        ui.checkbox(&mut st.confirm, tr(l, "Je confirme le remplacement des fichiers existants."));
    }

    // Lancement / progression
    let running = st.job.as_ref().map(|j| !j.finished.load(Ordering::SeqCst)).unwrap_or(false);
    if let Some(j) = st.job.clone() {
        let done = j.done.load(Ordering::SeqCst);
        ui.add(egui::ProgressBar::new(if j.total > 0 { done as f32 / j.total as f32 } else { 1.0 }).text(format!("{done}/{}", j.total)));
        if running {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
            if ui.button(tr(l, "Annuler")).clicked() {
                j.cancel.store(true, Ordering::SeqCst);
            }
        } else {
            let errs = j.errors.lock().map(|v| v.clone()).unwrap_or_default();
            st.msg = format!(
                "{} {}{}",
                trf(l, "{n} image(s) traitée(s)", &[("{n}", j.ok.load(Ordering::SeqCst).to_string())]),
                if j.skipped.load(Ordering::SeqCst) > 0 { trf(l, "· {n} ignorée(s) ", &[("{n}", j.skipped.load(Ordering::SeqCst).to_string())]) } else { String::new() },
                if errs.is_empty() { String::new() } else { trf(l, "· {n} erreur(s) : ", &[("{n}", errs.len().to_string())]) + &errs.join(" | ") }
            );
            st.job = None;
        }
    }
    let ready = !st.files.is_empty() && !running && (!need_confirm || st.confirm) && (!st.resize_on || st.mode > 0 || st.w > 0.0 || st.h > 0.0);
    if ui.add_enabled(ready, egui::Button::new(RichText::new(tr(l, "Traiter les images")).color(Color32::WHITE)).fill(accent())).clicked() {
        st.msg.clear();
        st.start();
    }
    if !st.msg.is_empty() {
        ui.label(RichText::new(&st.msg).weak());
    }
}
