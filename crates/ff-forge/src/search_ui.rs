//! Fenêtre de recherche multicritères de FileForge. v0.10.0
use crate::pane::{human, Loc};
use crate::search::{self, Hit, Kind, Progress, Query};
use crate::{fmt_time, App};
use eframe::egui::{self, RichText};
use ff_core::media::{is_doc, is_mesh, kind_of, MediaKind};
use ff_kit::icons::{self, Icon};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{mpsc::Receiver, Arc};
use std::time::Instant;

const KINDS: &[(&str, &str, Option<Kind>)] = &[
    ("Tous types", "All types", None),
    ("Images", "Images", Some(Kind::Image)),
    ("Vidéos", "Videos", Some(Kind::Video)),
    ("Audio", "Audio", Some(Kind::Audio)),
    ("Documents", "Documents", Some(Kind::Doc)),
    ("Archives", "Archives", Some(Kind::Archive)),
    ("Code", "Code", Some(Kind::Code)),
    ("Texte", "Text", Some(Kind::Text)),
    ("Dossiers", "Folders", Some(Kind::Dir)),
];
const DATES: &[(&str, &str, &str)] = &[("Toute date", "Any date", ""), ("24 heures", "24 hours", "24h"), ("7 jours", "7 days", "7d"), ("30 jours", "30 days", "30d"), ("1 an", "1 year", "1y")];

pub struct SearchUi {
    pub open: bool,
    text: String,
    root: String,
    content: String,
    size_min: String,
    size_max: String,
    kind: usize,
    date: usize,
    from: String,
    to: String,
    hidden: bool,
    fuzzy: bool,
    recursive: bool,
    all_drives: bool,
    rx: Option<Receiver<Hit>>,
    prog: Option<Arc<Progress>>,
    hits: Vec<Hit>,
    sel: Option<usize>,
    started: Instant,
    elapsed: f32,
    focus: bool,
}

impl Default for SearchUi {
    fn default() -> Self {
        SearchUi { open: false, text: String::new(), root: String::new(), content: String::new(), size_min: String::new(), size_max: String::new(), kind: 0, date: 0, from: String::new(), to: String::new(), hidden: false, fuzzy: true, recursive: true, all_drives: false, rx: None, prog: None, hits: vec![], sel: None, started: Instant::now(), elapsed: 0.0, focus: false }
    }
}

fn hit_icon(h: &Hit) -> Icon {
    if h.is_dir {
        return Icon::Folder;
    }
    let p: &Path = &h.path;
    match kind_of(p) {
        Some(MediaKind::Video) => Icon::Film,
        Some(MediaKind::Audio) => Icon::Note,
        Some(MediaKind::Image) => Icon::Picture,
        None if is_doc(p) || is_mesh(p) => Icon::Picture,
        None => Icon::Doc,
    }
}

impl SearchUi {
    fn stop(&mut self) {
        if let Some(p) = &self.prog {
            p.cancel.store(true, Ordering::Relaxed);
        }
        self.rx = None;
    }

    fn query(&self) -> Query {
        let mut q = Query::parse(&self.text);
        q.fuzzy = self.fuzzy;
        q.hidden = self.hidden;
        q.recursive = self.recursive;
        if let Some((_, _, Some(k))) = KINDS.get(self.kind) {
            q.kind = Some(*k);
        }
        if !self.content.trim().is_empty() {
            q.content = Some(search::norm(self.content.trim()));
        }
        if let Some(n) = search::parse_size(self.size_min.trim()).or_else(|| self.size_min.trim().parse().ok()) {
            q.size_min = Some(n);
        }
        if let Some(n) = search::parse_size(self.size_max.trim()).or_else(|| self.size_max.trim().parse().ok()) {
            q.size_max = Some(n);
        }
        if self.date > 0 {
            let spec = DATES[self.date].2;
            let q2 = Query::parse(&format!("date:{spec}"));
            q.after = q2.after;
        }
        // Période explicite : « du » (début inclus) « au » (fin de période incluse) ; AAAA, AAAA-MM, AAAA-MM-JJ ou JJ/MM/AAAA
        if let Some(a) = search::parse_date(self.from.trim()) {
            q.after = Some(a);
        }
        if let Some(b) = search::parse_date_end(self.to.trim()) {
            q.before = Some(b);
        }
        q
    }

    fn launch(&mut self) {
        self.stop();
        let q = self.query();
        self.hits.clear();
        self.sel = None;
        if q.is_empty() {
            return;
        }
        let roots = if self.all_drives { search::all_roots() } else { vec![PathBuf::from(self.root.trim())] };
        let (rx, prog) = search::start_multi(roots, q);
        self.rx = Some(rx);
        self.prog = Some(prog);
        self.started = Instant::now();
        self.elapsed = 0.0;
    }

    /// Récupère les résultats arrivés ; trie à la fin.
    fn poll(&mut self) -> bool {
        let mut running = false;
        if let Some(rx) = &self.rx {
            while let Ok(h) = rx.try_recv() {
                self.hits.push(h);
            }
            let done = self.prog.as_ref().map(|p| p.done.load(Ordering::SeqCst) || p.cancel.load(Ordering::Relaxed)).unwrap_or(true);
            running = !done;
            self.elapsed = self.started.elapsed().as_secs_f32();
            if done {
                while let Ok(h) = rx.try_recv() {
                    self.hits.push(h);
                }
                self.hits.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.path.cmp(&b.path)));
                self.rx = None;
            }
        }
        running
    }
}

impl App {
    pub fn open_search(&mut self) {
        let a = self.active;
        if let Some(d) = self.panes[a].disk_dir() {
            if self.search.root.is_empty() || !self.search.open {
                self.search.root = d.to_string_lossy().to_string();
            }
        }
        self.search.open = true;
        self.search.focus = true;
    }

    /// Affiche le fichier dans le panneau actif (dossier parent + curseur).
    fn reveal(&mut self, path: &Path) -> Option<usize> {
        let a = self.active;
        let (dir, name) = if path.is_dir() { (path.to_path_buf(), None) } else { (path.parent()?.to_path_buf(), path.file_name().map(|n| n.to_string_lossy().to_string())) };
        self.panes[a].goto(Loc::Disk(dir));
        let name = name?;
        let i = self.panes[a].rows.iter().position(|r| r.name == name)?;
        self.panes[a].cursor = Some(i);
        self.panes[a].scroll_to_cursor = true;
        self.panes[a].sel.clear();
        self.panes[a].sel.insert(self.panes[a].rows[i].full.clone());
        Some(i)
    }

    pub fn search_window(&mut self, ctx: &egui::Context) {
        if !self.search.open {
            return;
        }
        let running = self.search.poll();
        if running {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        let en = self.en();
        let t = |fr: &'static str, e: &'static str| if en { e } else { fr };
        let mut open = true;
        let mut go = false;
        let mut stop = false;
        let mut action: Option<(PathBuf, bool)> = None; // (chemin, ouvrir ?)
        let mut pick_root = false;
        let pal = ff_kit::theme::current();
        egui::Window::new(t("Rechercher", "Search")).open(&mut open).default_size([760.0, 560.0]).resizable(true).show(ctx, |ui| {
            ff_kit::help::button(ui, "fileforge", "recherche", "?");
            let s = &mut self.search;
            // Largeur de référence figée : évite que la fenêtre s'élargisse à chaque image
            let w0 = ui.available_width();
            ui.set_max_width(w0);
            ui.horizontal(|ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut s.text).desired_width((w0 - 260.0).max(120.0)).hint_text(t("nom, *.jpg, .pdf, size:>5mb, date:7d, contenu:mot…", "name, *.jpg, .pdf, size:>5mb, date:7d, content:word…")));
                if s.focus {
                    r.request_focus();
                    s.focus = false;
                }
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    go = true;
                }
                if running {
                    if ui.button(t("Arrêter", "Stop")).clicked() {
                        stop = true;
                    }
                    ui.spinner();
                } else if ui.button(RichText::new(t("Rechercher", "Search")).strong()).clicked() {
                    go = true;
                }
            });
            ui.horizontal(|ui| {
                ui.label(t("Dans :", "In:"));
                ui.add_enabled(!s.all_drives, egui::TextEdit::singleline(&mut s.root).desired_width((w0 - 150.0).max(120.0)));
                if ui.add_enabled(!s.all_drives, egui::Button::new(t("Parcourir…", "Browse…"))).clicked() {
                    pick_root = true;
                }
            });
            ui.horizontal_wrapped(|ui| {
                egui::ComboBox::from_id_salt("sk").selected_text(if en { KINDS[s.kind].1 } else { KINDS[s.kind].0 }).show_ui(ui, |ui| {
                    for (i, k) in KINDS.iter().enumerate() {
                        ui.selectable_value(&mut s.kind, i, if en { k.1 } else { k.0 });
                    }
                });
                egui::ComboBox::from_id_salt("sd").selected_text(if en { DATES[s.date].1 } else { DATES[s.date].0 }).show_ui(ui, |ui| {
                    for (i, d) in DATES.iter().enumerate() {
                        ui.selectable_value(&mut s.date, i, if en { d.1 } else { d.0 });
                    }
                });
                ui.label(t("Taille ≥", "Size ≥"));
                ui.add(egui::TextEdit::singleline(&mut s.size_min).desired_width(60.0).hint_text("1mb"));
                ui.label("≤");
                ui.add(egui::TextEdit::singleline(&mut s.size_max).desired_width(60.0).hint_text("2gb"));
                ui.label(t("Contient :", "Contains:"));
                ui.add(egui::TextEdit::singleline(&mut s.content).desired_width(130.0).hint_text(t("texte dans le fichier", "text inside file")));
            });
            ui.horizontal_wrapped(|ui| {
                ui.label(t("Période : du", "Period: from"));
                ui.add(egui::TextEdit::singleline(&mut s.from).desired_width(90.0).hint_text("2023"));
                ui.label(t("au", "to"));
                ui.add(egui::TextEdit::singleline(&mut s.to).desired_width(90.0).hint_text("2024-06"));
                ui.label(RichText::new(t("AAAA, AAAA-MM ou AAAA-MM-JJ (prioritaire sur « Toute date »)", "YYYY, YYYY-MM or YYYY-MM-DD (overrides the date list)")).small().color(pal.weak));
            });
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut s.recursive, t("Sous-dossiers", "Subfolders"));
                ui.checkbox(&mut s.all_drives, t("Tous les lecteurs", "All drives")).on_hover_text(t("Cherche sur tous les disques (locaux, amovibles et lecteurs réseau mappés) ; peut être long.", "Searches every drive (local, removable and mapped network drives); may take a while."));
                ui.checkbox(&mut s.fuzzy, t("Approximatif", "Fuzzy"));
                ui.checkbox(&mut s.hidden, t("Fichiers cachés", "Hidden files"));
            });
            ui.collapsing(t("Aide à la recherche", "Search help"), |ui| {
                ui.label(RichText::new(t(
                    "Mots libres : tous doivent figurer dans le nom (ou le dossier). * et ? = jokers, \"phrase\", -mot exclut.\n.pdf ou ext:pdf,docx · type:image|video|audio|doc|archive|code|dossier\nsize:>10mb · size:1mb..5mb · date:>2024-01-31 · date:7d (h, d, w, m, y) · contenu:\"deux mots\"\nExemple : facture -brouillon .pdf date:1y size:>100k",
                    "Free words: all must appear in the name (or folder). * and ? wildcards, \"phrase\", -word excludes.\n.pdf or ext:pdf,docx · type:image|video|audio|doc|archive|code|folder\nsize:>10mb · size:1mb..5mb · date:>2024-01-31 · date:7d (h, d, w, m, y) · content:\"two words\"\nExample: invoice -draft .pdf date:1y size:>100k",
                )).small());
            });
            ui.separator();
            let n = s.hits.len();
            let scanned = s.prog.as_ref().map(|p| p.scanned.load(Ordering::Relaxed)).unwrap_or(0);
            ui.label(RichText::new(format!("{} {} · {} {} · {:.1} s{}", n, t("résultat(s)", "result(s)"), scanned, t("éléments parcourus", "items scanned"), s.elapsed, if n >= search::MAX_RESULTS { " (limite)" } else { "" })).small().color(pal.weak));
            let h = (ui.available_height() - 34.0).max(80.0);
            egui::ScrollArea::vertical().id_salt("sres").max_height(h).auto_shrink([false, false]).show_rows(ui, 34.0, n, |ui, range| {
                for i in range {
                    let hit = &s.hits[i];
                    let name = hit.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| hit.path.to_string_lossy().to_string());
                    let dir = hit.path.parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
                    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 32.0), egui::Sense::click());
                    if s.sel == Some(i) {
                        ui.painter().rect_filled(rect, 3.0, pal.accent.gamma_multiply(0.25));
                    } else if resp.hovered() {
                        ui.painter().rect_filled(rect, 3.0, pal.accent.gamma_multiply(0.1));
                    }
                    let mut c = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(egui::vec2(4.0, 1.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
                    let (ir, _) = c.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
                    icons::paint(c.painter(), ir, hit_icon(hit), pal.text);
                    c.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.add(egui::Label::new(RichText::new(name).strong()).truncate());
                        let sub = match &hit.snippet {
                            Some(sn) => sn.clone(),
                            None => dir,
                        };
                        ui.add(egui::Label::new(RichText::new(sub).small().color(pal.weak)).truncate());
                    });
                    c.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(format!("{}  {}", if hit.is_dir { String::new() } else { human(hit.size) }, fmt_time(hit.mtime))).small().color(pal.weak));
                    });
                    if resp.clicked() {
                        s.sel = Some(i);
                    }
                    if resp.double_clicked() {
                        action = Some((hit.path.clone(), !hit.is_dir));
                    }
                }
            });
            ui.horizontal(|ui| {
                let cur = s.sel.and_then(|i| s.hits.get(i)).map(|h| (h.path.clone(), h.is_dir));
                ui.add_enabled_ui(cur.is_some(), |ui| {
                    if ui.button(t("Afficher dans le panneau", "Show in pane")).clicked() {
                        action = cur.clone().map(|(p, _)| (p, false));
                    }
                    if ui.button(t("Ouvrir", "Open")).clicked() {
                        action = cur.clone().map(|(p, d)| (p, !d));
                    }
                    if ui.button(t("Copier le chemin", "Copy path")).clicked() {
                        if let Some((p, _)) = &cur {
                            ui.ctx().copy_text(p.to_string_lossy().to_string());
                        }
                    }
                });
            });
        });
        if pick_root {
            if let Some(d) = rfd::FileDialog::new().pick_folder() {
                self.search.root = d.to_string_lossy().to_string();
            }
        }
        if stop {
            self.search.stop();
        }
        if go {
            if !self.search.all_drives && !Path::new(self.search.root.trim()).is_dir() {
                self.say(self.t("Dossier de recherche introuvable.", "Search folder not found.").to_string());
            } else {
                self.search.launch();
            }
        }
        if let Some((p, opn)) = action {
            if let Some(i) = self.reveal(&p) {
                if opn {
                    self.open_row(i);
                }
            }
        }
        if !open {
            self.search.stop();
            self.search.open = false;
        }
    }
}
