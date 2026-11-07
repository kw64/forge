//! FileForge — gestionnaire de fichiers double panneau (esprit FileMaster/Amiga). v0.12.0
//! Les médias s'ouvrent dans MediaForge Player, la conversion passe par MediaForge.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod archive;
mod fsops;
mod jobs;
mod shell;
mod pane;
mod rename;
mod rename_ui;
mod search;
mod search_ui;

use eframe::egui::{self, Color32, RichText};
use ff_core::media::{is_doc, is_mesh, kind_of, MediaKind};
use ff_core::Lang;
use ff_kit::icons::{self, Icon};
use pane::{Loc, Pane, Row};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const VERSION: &str = env!("CARGO_PKG_VERSION");

enum Input {
    MakeDir,
    CopyAs,
    Rename,
}

enum Dialog {
    Confirm { text: String, rows: Vec<Row>, pane: usize },
    Input { kind: Input, title: String, text: String, rows: Vec<Row>, pane: usize },
    Info(String),
    /// Création d'archive : format, nom et mot de passe facultatif.
    Pack { name: String, fmt: usize, pw: String, pw2: String, show: bool, rows: Vec<Row>, pane: usize },
    /// Mot de passe d'une archive chiffrée (ouverture / extraction).
    Password { path: PathBuf, pw: String, bad: bool },
}

/// Action à relancer une fois le mot de passe saisi.
enum Retry {
    Open(usize),
    Button(String),
}

struct Viewer {
    title: String,
    text: String,
    hex: bool,
}

struct App {
    panes: [Pane; 2],
    active: usize,
    dialog: Option<Dialog>,
    viewer: Option<Viewer>,
    settings: ff_profiles::Settings,
    show_settings: bool,
    msg: String,
    tmp: PathBuf,
    tmp_n: usize,
    theme_check: std::time::Instant,
    search: search_ui::SearchUi,
    rename: rename_ui::RenameUi,
    retry: Option<Retry>,
    jobs: jobs::Jobs,
    /// Action à relancer si la tâche de ce type demande un mot de passe.
    pw_retry: std::collections::HashMap<&'static str, Retry>,
    show_queue: bool,
    /// Saisie incrémentale : lettres tapées et heure de la dernière frappe.
    typeahead: (String, std::time::Instant),
    /// Glisser-déposer interne : panneau source + chemins glissés ; rectangles des panneaux.
    drag: Option<(usize, Vec<PathBuf>, usize)>,
    pane_rects: [egui::Rect; 2],
    outside: ff_kit::outside::OutsideClose,
    ctx_cache: Option<egui::Context>,
}

fn exe_beside(name: &str) -> PathBuf {
    let f = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join(&f))).filter(|p| p.exists()).unwrap_or_else(|| PathBuf::from(f))
}

#[cfg(windows)]
static DRAGGING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static DRAG_DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Glisser-déposer sortant (OLE) : exécuté dans le thread de la fenêtre, déclenché par un message Windows.
#[cfg(windows)]
mod oledrag {
    use raw_window_handle::{HandleError, HasWindowHandle, RawWindowHandle, Win32WindowHandle, WindowHandle};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicIsize, Ordering};
    use std::sync::Mutex;

    const WM_DRAG: u32 = 0x8000 + 0x44; // WM_APP + 0x44
    const GWLP_WNDPROC: i32 = -4;
    static ORIG: AtomicIsize = AtomicIsize::new(0);
    static PENDING: Mutex<Option<Vec<PathBuf>>> = Mutex::new(None);

    #[link(name = "user32")]
    extern "system" {
        fn SetWindowLongPtrW(h: isize, i: i32, v: isize) -> isize;
        fn CallWindowProcW(p: isize, h: isize, m: u32, w: usize, l: isize) -> isize;
        fn PostMessageW(h: isize, m: u32, w: usize, l: isize) -> i32;
    }

    struct Hw(isize);
    impl HasWindowHandle for Hw {
        fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
            let n = std::num::NonZeroIsize::new(self.0).ok_or(HandleError::Unavailable)?;
            // SAFETY : la fenêtre principale vit pendant toute la durée du glisser.
            Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::Win32(Win32WindowHandle::new(n))) })
        }
    }

    unsafe extern "system" fn proc(h: isize, m: u32, w: usize, l: isize) -> isize {
        if m == WM_DRAG {
            let files = PENDING.lock().ok().and_then(|mut p| p.take());
            if let Some(files) = files {
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let _ = drag::start_drag(&Hw(h), drag::DragItem::Files(files), drag::Image::Raw(include_bytes!("../../../assets/fileforge.png").to_vec()), |_, _| {}, drag::Options::default());
                }));
            }
            super::DRAGGING.store(false, Ordering::SeqCst);
            super::DRAG_DONE.store(true, Ordering::SeqCst);
            return 0;
        }
        CallWindowProcW(ORIG.load(Ordering::SeqCst), h, m, w, l)
    }

    /// Prépare le glisser et le déclenche via la boucle de messages de la fenêtre.
    pub fn start(hwnd: isize, files: Vec<PathBuf>) {
        unsafe {
            if ORIG.load(Ordering::SeqCst) == 0 {
                let o = SetWindowLongPtrW(hwnd, GWLP_WNDPROC, proc as *const () as usize as isize);
                ORIG.store(o, Ordering::SeqCst);
            }
            if let Ok(mut p) = PENDING.lock() {
                *p = Some(files);
            }
            if PostMessageW(hwnd, WM_DRAG, 0, 0) == 0 {
                super::DRAGGING.store(false, Ordering::SeqCst);
            }
        }
    }
}

fn launch(name: &str, files: &[PathBuf]) -> Result<(), String> {
    std::process::Command::new(exe_beside(name)).args(files).spawn().map(|_| ()).map_err(|e| format!("{name} : {e}"))
}

fn open_os(p: &Path) {
    #[cfg(windows)]
    let _ = std::process::Command::new("cmd").args(["/C", "start", ""]).arg(p).spawn();
    #[cfg(not(windows))]
    let _ = std::process::Command::new("xdg-open").arg(p).spawn();
}

fn edit_os(p: &Path) {
    #[cfg(windows)]
    let _ = std::process::Command::new("notepad").arg(p).spawn();
    #[cfg(not(windows))]
    open_os(p);
}

fn fmt_time(t: Option<std::time::SystemTime>) -> String {
    let Some(t) = t else { return String::new() };
    let s = t.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let (days, rem) = (s.div_euclid(86400), s.rem_euclid(86400));
    // Date civile depuis le nombre de jours (algorithme de H. Hinnant).
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}/{m:02}/{d:02} {:02}:{:02}", rem / 3600, rem % 3600 / 60)
}

fn hexdump(b: &[u8]) -> String {
    let mut o = String::new();
    for (i, c) in b.chunks(16).enumerate() {
        o.push_str(&format!("{:08X}  ", i * 16));
        for j in 0..16 {
            match c.get(j) {
                Some(x) => o.push_str(&format!("{x:02X} ")),
                None => o.push_str("   "),
            }
            if j == 7 {
                o.push(' ');
            }
        }
        o.push_str(" |");
        o.extend(c.iter().map(|&x| if (32..127).contains(&x) { x as char } else { '.' }));
        o.push_str("|\n");
    }
    o
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, start: Option<PathBuf>) -> Self {
        let settings = ff_profiles::load();
        ff_kit::theme::apply(&cc.egui_ctx, ff_kit::theme::Style::from_id(&settings.theme), settings.dark);
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let left = start.unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| home.clone()));
        let tmp = std::env::temp_dir().join(format!("fileforge-{}", std::process::id()));
        let en = settings.lang == Lang::En;
        let mut p0 = Pane::new(if left.is_dir() { left.clone() } else { left.parent().map(|x| x.to_path_buf()).unwrap_or_default() });
        if left.is_file() {
            // Archive passée en argument (« Ouvrir avec FileForge ») : ouverte directement.
            if let Ok(a) = archive::Archive::open(&left) {
                let back = left.parent().map(|x| x.to_path_buf()).unwrap_or_default();
                p0.goto(Loc::Arc { arch: std::sync::Arc::new(a), inner: String::new(), back });
            }
        }
        let mut p1 = Pane::new(dirs::download_dir().unwrap_or(home));
        for p in [&mut p0, &mut p1] {
            p.en = en;
            p.hidden = settings.hidden_places.clone();
            p.custom = settings.custom_places.clone();
        }
        App {
            panes: [p0, p1],
            active: 0,
            dialog: None,
            viewer: None,
            settings,
            show_settings: false,
            msg: String::new(),
            tmp,
            tmp_n: 0,
            theme_check: std::time::Instant::now(),
            search: Default::default(),
            rename: Default::default(),
            retry: None,
            jobs: Default::default(),
            pw_retry: Default::default(),
            show_queue: false,
            typeahead: (String::new(), std::time::Instant::now()),
            drag: None,
            pane_rects: [egui::Rect::NOTHING; 2],
            outside: Default::default(),
            ctx_cache: None,
        }
    }

    fn en(&self) -> bool {
        self.settings.lang == Lang::En
    }
    fn t<'a>(&self, fr: &'a str, en: &'a str) -> &'a str {
        if self.en() { en } else { fr }
    }
    fn retro(&self) -> bool {
        self.settings.theme == "amiga"
    }
    fn say(&mut self, s: impl Into<String>) {
        let s: String = s.into();
        if archive::is_pw_error(&s) {
            return; // géré par la fenêtre de mot de passe
        }
        self.msg = s;
    }

    /// Applique la liste des dossiers de « Lecteurs » aux deux panneaux.
    fn sync_places(&mut self) {
        for p in &mut self.panes {
            p.hidden = self.settings.hidden_places.clone();
            p.custom = self.settings.custom_places.clone();
            if matches!(p.loc, Loc::Drives) {
                p.refresh();
            }
        }
    }

    fn other(&self) -> usize {
        1 - self.active
    }

    /// Chemins disque des lignes ; les entrées d'archive sont d'abord extraites dans un dossier temporaire.
    fn materialize(&mut self, pi: usize, rows: &[Row]) -> Result<Vec<PathBuf>, String> {
        if let Loc::Arc { arch, inner, .. } = self.panes[pi].loc.clone() {
            self.tmp_n += 1;
            let d = self.tmp.join(self.tmp_n.to_string());
            let names: Vec<String> = rows.iter().map(|r| r.full.clone()).collect();
            let pred = move |n: &str| names.iter().any(|s| n == s || n.starts_with(&format!("{s}/")));
            arch.extract(&pred, &d, &inner)?;
            Ok(rows.iter().map(|r| d.join(&r.name)).collect())
        } else {
            Ok(rows.iter().filter_map(|r| self.panes[pi].disk_path(r)).collect())
        }
    }

    fn refresh_all(&mut self) {
        for p in &mut self.panes {
            if matches!(p.loc, Loc::Disk(_)) {
                let (c, s) = (p.cursor, p.sel.clone());
                p.refresh();
                p.cursor = c.map(|c| c.min(p.rows.len().saturating_sub(1)));
                p.sel = s.into_iter().filter(|n| p.rows.iter().any(|r| &r.full == n)).collect();
            }
        }
    }

    fn open_row(&mut self, i: usize) {
        self.retry = Some(Retry::Open(i));
        let a = self.active;
        if let Some(path) = self.panes[a].enter(i) {
            let k = kind_of(&path);
            if k.is_some() || is_doc(&path) || is_mesh(&path) {
                if let Err(e) = launch("mediaforge-player", &[path]) {
                    self.say(e);
                }
            } else {
                open_os(&path);
            }
        } else if let Some(r) = self.panes[a].rows.get(i).cloned() {
            // Fichier dans une archive : extrait puis ouvert.
            if matches!(self.panes[a].loc, Loc::Arc { .. }) && !r.is_dir && !r.is_parent {
                match self.materialize(a, &[r]) {
                    Ok(p) => {
                        if kind_of(&p[0]).is_some() || is_doc(&p[0]) || is_mesh(&p[0]) {
                            let _ = launch("mediaforge-player", &p);
                        } else {
                            open_os(&p[0]);
                        }
                    }
                    Err(e) => self.say(e),
                }
            }
        }
    }

    /// Fenêtre de gestion de la file d'attente : en cours, en attente (monter/annuler), historique.
    fn queue_window(&mut self, ctx: &egui::Context) {
        if !self.show_queue {
            return;
        }
        let mut open = true;
        let mut cancel: Option<Arc<jobs::Job>> = None;
        let mut up: Option<Arc<jobs::Job>> = None;
        let (mut cancel_all, mut clear) = (false, false);
        let (tq, ta, tu, tc, tca, thi, tcl, tnone) = (self.t("File d'attente", "Queue"), self.t("Annuler", "Cancel"), self.t("Monter", "Move up"), self.t("Annuler", "Cancel"), self.t("Tout annuler", "Cancel all"), self.t("Terminées", "Finished"), self.t("Vider l'historique", "Clear history"), self.t("Aucune action en file.", "No queued action."));
        let _ = (ta, tc);
        let pal = ff_kit::theme::current();
        egui::Window::new(tq).open(&mut open).default_width(460.0).show(ctx, |ui| {
            ff_kit::help::button(ui, "fileforge", "file-attente", "?");
            let act: Vec<Arc<jobs::Job>> = self.jobs.active().iter().filter(|j| jobs::queued(j.key)).cloned().collect();
            if act.is_empty() {
                ui.label(RichText::new(tnone).color(pal.weak));
            }
            for (i, j) in act.iter().enumerate() {
                ui.horizontal(|ui| {
                    if j.pending() {
                        ui.label(RichText::new(format!("{}.", i + 1)).color(pal.weak));
                    } else {
                        ui.spinner();
                    }
                    ui.add(egui::Label::new(&j.label).truncate());
                    if !j.pending() {
                        if let Some(f) = j.fraction() {
                            ui.add(egui::ProgressBar::new(f).desired_width(90.0).show_percentage());
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button(tc).clicked() {
                            cancel = Some(j.clone());
                        }
                        if j.pending() && i > 0 && ui.small_button(tu).clicked() {
                            up = Some(j.clone());
                        }
                    });
                });
            }
            if act.len() > 1 && ui.button(tca).clicked() {
                cancel_all = true;
            }
            if !self.jobs.history.is_empty() {
                ui.separator();
                ui.label(RichText::new(thi).strong());
                egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
                    for (l, ok, m) in self.jobs.history.iter().rev() {
                        let c = if *ok { pal.ok } else { pal.err };
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(if *ok { "✔" } else { "✖" }).color(c));
                            ui.add(egui::Label::new(format!("{l} — {m}")).truncate());
                        });
                    }
                });
                if ui.button(tcl).clicked() {
                    clear = true;
                }
            }
        });
        if let Some(j) = cancel {
            self.jobs.cancel(&j);
        }
        if let Some(j) = up {
            self.jobs.move_up(&j);
        }
        if cancel_all {
            self.jobs.cancel_all();
        }
        if clear {
            self.jobs.history.clear();
        }
        self.show_queue = open;
    }

    /// Panneau (0/1) sous le pointeur.
    fn pane_at(&self, pos: egui::Pos2) -> Option<usize> {
        self.pane_rects.iter().position(|r| r.contains(pos))
    }

    /// Met en file des copies/déplacements de chemins disque vers `dst`.
    fn enqueue_paths(&mut self, paths: Vec<PathBuf>, dst: PathBuf, mv: bool) {
        let pairs: Vec<(PathBuf, PathBuf)> = paths.into_iter().filter_map(|f| {
            let to = dst.join(f.file_name()?);
            (to != f && !dst.starts_with(&f)).then_some((f, to))
        }).collect();
        if pairs.is_empty() {
            return;
        }
        let en = self.en();
        let key: &'static str = if mv { "move" } else { "copy" };
        let label = format!("{} → {}", if mv { self.t("Déplacement", "Moving") } else { self.t("Copie", "Copying") }, dst.display());
        self.jobs.spawn(key, label, move |job| {
            job.set_total(pairs.iter().map(|(f, _)| fsops::tree_size(f)).sum::<u64>().max(1));
            for (from, to) in &pairs {
                if mv { fsops::move_p(from, to, job)? } else { fsops::copy_p(from, to, job)? }
            }
            Ok(format!("{} {}", pairs.len(), if en { "item(s) processed" } else { "élément(s) traité(s)" }))
        });
    }

    /// Glisser-déposer : fichiers venant de l'extérieur, entre panneaux, et vers l'extérieur.
    fn drag_and_drop(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        let _ = frame;
        // Fin d'un glisser sortant : rafraîchir les panneaux (un déplacement a pu modifier les dossiers).
        if DRAG_DONE.swap(false, std::sync::atomic::Ordering::SeqCst) {
            self.refresh_all();
        }
        // 1) Dépôt depuis l'explorateur : copie vers le dossier du panneau visé (Maj = déplacement).
        let (dropped, pos, shift) = ctx.input(|i| (i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect::<Vec<_>>(), i.pointer.latest_pos(), i.modifiers.shift));
        if !dropped.is_empty() {
            let pane = pos.and_then(|p| self.pane_at(p)).unwrap_or(self.active);
            match self.panes[pane].disk_dir().map(|p| p.to_path_buf()) {
                Some(dst) => self.enqueue_paths(dropped, dst, shift),
                None => self.say(self.t("Destination : un dossier disque est requis.", "Destination must be a disk folder.").to_string()),
            }
        }
        // 2) Glisser interne / sortant.
        let Some((src, paths, n)) = self.drag.clone() else { return };
        let (down, latest, screen) = ctx.input(|i| (i.pointer.any_down(), i.pointer.latest_pos(), i.content_rect()));
        let shift = ctx.input(|i| i.modifiers.shift);
        if down {
            if let Some(p) = latest.filter(|p| screen.contains(*p)) {
                let tgt = self.pane_at(p).filter(|t| *t != src);
                let txt = format!("{} {} {}", n, self.t("élément(s)", "item(s)"), if tgt.is_some() { if shift { self.t("→ déplacer", "→ move") } else { self.t("→ copier", "→ copy") } } else { "" });
                egui::Area::new(egui::Id::unique("dragtip")).order(egui::Order::Tooltip).fixed_pos(p + egui::vec2(14.0, 14.0)).interactable(false).show(ctx, |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.label(txt);
                    });
                });
                ctx.set_cursor_icon(if tgt.is_some() { egui::CursorIcon::Copy } else { egui::CursorIcon::Grabbing });
                ctx.request_repaint();
                return;
            }
            // Pointeur sorti de la fenêtre : glisser natif vers l'explorateur (Windows).
            #[cfg(windows)]
            {
                self.drag = None;
                if !paths.is_empty() {
                    // Le glisser OLE ouvre une boucle de messages modale : il ne doit pas être lancé depuis le rappel
                    // d'affichage (ré-entrance). On poste un message à la fenêtre ; il est traité par oledrag::proc,
                    // hors de tout rappel eframe, sur le thread de la fenêtre (capture souris cohérente).
                    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
                    if let Ok(RawWindowHandle::Win32(h)) = frame.window_handle().map(|h| h.as_raw()) {
                        if !DRAGGING.swap(true, std::sync::atomic::Ordering::SeqCst) {
                            oledrag::start(h.hwnd.get(), paths);
                        }
                    }
                }
                return;
            }
            #[cfg(not(windows))]
            return;
        }
        // Bouton relâché dans la fenêtre : dépôt sur l'autre panneau.
        self.drag = None;
        if let Some(t) = latest.and_then(|p| self.pane_at(p)).filter(|t| *t != src) {
            if let Some(dst) = self.panes[t].disk_dir().map(|p| p.to_path_buf()) {
                self.enqueue_paths(paths, dst, shift);
            } else {
                self.say(self.t("Destination : un dossier disque est requis.", "Destination must be a disk folder.").to_string());
            }
        }
    }

    fn busy(&mut self) {
        self.say(self.t("Cette opération est déjà en cours.", "This operation is already running.").to_string());
    }

    /// Copie / déplacement en tâche de fond (progression, annulation).
    fn do_copy(&mut self, mv: bool, new_name: Option<String>) {
        let (s, d) = (self.active, self.other());
        let rows = self.panes[s].targets();
        if rows.is_empty() {
            return;
        }
        let Some(dst) = self.panes[d].disk_dir().map(|p| p.to_path_buf()) else {
            self.say(self.t("Destination : un dossier disque est requis.", "Destination must be a disk folder.").to_string());
            return;
        };
        let key: &'static str = if mv { "move" } else { "copy" };
        let en = self.en();
        let label = format!("{} → {}", if mv { self.t("Déplacement", "Moving") } else { self.t("Copie", "Copying") }, dst.display());
        let done_msg = move |n: usize| format!("{n} {}", if en { "item(s) processed" } else { "élément(s) traité(s)" });
        if let Loc::Arc { arch, inner, .. } = self.panes[s].loc.clone() {
            let names: Vec<String> = rows.iter().map(|r| r.full.clone()).collect();
            self.pw_retry.insert(key, Retry::Button(key.to_string()));
            self.jobs.spawn(key, label, move |job| {
                let pred = |n: &str| names.iter().any(|x| n == x || n.starts_with(&format!("{x}/")));
                job.set_total(arch.size_of(&pred).max(1));
                arch.extract_p(&pred, &dst, &inner, &|b| job.step(b)).map(done_msg)
            });
            return;
        }
        let mut pairs: Vec<(PathBuf, PathBuf)> = Vec::new();
        for r in &rows {
            let Some(from) = self.panes[s].disk_path(r) else {
                return self.say("source invalide");
            };
            let to = dst.join(new_name.clone().unwrap_or_else(|| r.name.clone()));
            if to == from {
                return self.say(self.t("Source et destination identiques.", "Source and destination are the same.").to_string());
            }
            pairs.push((from, to));
        }
        self.jobs.spawn(key, label, move |job| {
            job.set_total(pairs.iter().map(|(f, _)| fsops::tree_size(f)).sum::<u64>().max(1));
            for (from, to) in &pairs {
                if mv { fsops::move_p(from, to, job)? } else { fsops::copy_p(from, to, job)? }
            }
            Ok(done_msg(pairs.len()))
        });
    }

    /// Extraction d'archives en tâche de fond.
    fn unpack(&mut self) {
        let (s, d) = (self.active, self.other());
        let Some(dst) = self.panes[d].disk_dir().map(|p| p.to_path_buf()) else {
            self.say(self.t("Destination : un dossier disque est requis.", "Destination must be a disk folder.").to_string());
            return;
        };
        let en = self.en();
        let label = format!("{} → {}", self.t("Extraction", "Extracting"), dst.display());
        let msg = move |n: usize| format!("{n} {}", if en { "entries extracted" } else { "entrée(s) extraite(s)" });
        self.pw_retry.insert("unpack", Retry::Button("unpack".into()));
        if let Loc::Arc { arch, inner, .. } = self.panes[s].loc.clone() {
            let rows = self.panes[s].targets();
            let names: Vec<String> = rows.iter().map(|r| r.full.clone()).collect();
            self.jobs.spawn("unpack", label, move |job| {
                let all = names.is_empty();
                let pred = |n: &str| all || names.iter().any(|x| n == x || n.starts_with(&format!("{x}/")));
                job.set_total(arch.size_of(&pred).max(1));
                arch.extract_p(&pred, &dst, if all { "" } else { &inner }, &|b| job.step(b)).map(msg)
            });
            return;
        }
        let mut paths: Vec<PathBuf> = Vec::new();
        for r in self.panes[s].targets().into_iter().filter(|r| r.is_arc) {
            match self.panes[s].disk_path(&r) {
                Some(p) => paths.push(p),
                None => return self.say("source invalide"),
            }
        }
        self.jobs.spawn("unpack", label, move |job| {
            let mut opened = Vec::new();
            for p in &paths {
                opened.push((p.clone(), archive::Archive::open(p)?));
            }
            job.set_total(opened.iter().map(|(_, a)| a.size_of(&|_| true)).sum::<u64>().max(1));
            let mut n = 0;
            for (p, a) in &opened {
                let stem = p.file_stem().map(|x| x.to_string_lossy().into_owned()).unwrap_or_else(|| "archive".into());
                n += a.extract_p(&|_| true, &dst.join(stem), "", &|b| job.step(b))?;
            }
            Ok(msg(n))
        });
    }

    fn view(&mut self, hex: bool) {
        let a = self.active;
        let rows: Vec<Row> = self.panes[a].targets().into_iter().filter(|r| !r.is_dir).take(1).collect();
        let Some(r) = rows.first().cloned() else { return };
        match self.materialize(a, &[r.clone()]) {
            Ok(p) => {
                use std::io::Read;
                let lim = if hex { 64 * 1024 } else { 256 * 1024 };
                let mut buf = Vec::new();
                if let Err(e) = std::fs::File::open(&p[0]).and_then(|f| f.take(lim).read_to_end(&mut buf)) {
                    self.say(e.to_string());
                    return;
                }
                let text = if hex { hexdump(&buf) } else { String::from_utf8_lossy(&buf).replace('\r', "").chars().map(|c| if c.is_control() && c != '\n' && c != '\t' { '·' } else { c }).collect() };
                self.viewer = Some(Viewer { title: r.name, text, hex });
            }
            Err(e) => self.say(e),
        }
    }

    fn send_to(&mut self, app: &str, filter: impl Fn(&Path) -> bool) {
        let a = self.active;
        let rows: Vec<Row> = self.panes[a].targets().into_iter().filter(|r| !r.is_dir).collect();
        match self.materialize(a, &rows) {
            Ok(ps) => {
                let sel: Vec<PathBuf> = ps.iter().filter(|p| filter(p)).cloned().collect();
                let go = if sel.is_empty() { ps } else { sel };
                if go.is_empty() {
                    return;
                }
                if let Err(e) = launch(app, &go) {
                    self.say(e);
                }
            }
            Err(e) => self.say(e),
        }
    }

    fn run_dialog(&mut self, d: Dialog) {
        match d {
            Dialog::Confirm { rows, pane, .. } => {
                let paths: Vec<PathBuf> = rows.iter().filter_map(|r| self.panes[pane].disk_path(r)).collect();
                let en = self.en();
                let label = self.t("Suppression", "Deleting").to_string();
                self.jobs.spawn("delete", label, move |job| {
                    job.set_total(paths.len() as u64);
                    let mut n = 0;
                    for p in &paths {
                        job.set_detail(p.file_name().map(|x| x.to_string_lossy().to_string()).unwrap_or_default());
                        fsops::remove(p)?;
                        n += 1;
                        if !job.step(1) {
                            return Err(format!("Annulé ({n})"));
                        }
                    }
                    Ok(format!("{n} {}", if en { "item(s) deleted" } else { "élément(s) supprimé(s)" }))
                });
            }
            Dialog::Input { kind, text, rows, pane, .. } => {
                let text = text.trim().to_string();
                if text.is_empty() {
                    return;
                }
                let dir = self.panes[pane].disk_dir().map(|p| p.to_path_buf());
                match kind {
                    Input::MakeDir => match dir.map(|d| std::fs::create_dir_all(d.join(&text))) {
                        Some(Ok(_)) => self.say(self.t("Dossier créé.", "Folder created.").to_string()),
                        Some(Err(e)) => self.say(e.to_string()),
                        None => self.say(self.t("Création impossible ici.", "Cannot create here.").to_string()),
                    },
                    Input::Rename => {
                        if let (Some(r), Some(d)) = (rows.first(), dir) {
                            match std::fs::rename(d.join(&r.name), d.join(&text)) {
                                Ok(_) => self.say(self.t("Renommé.", "Renamed.").to_string()),
                                Err(e) => self.say(e.to_string()),
                            }
                        }
                    }
                    Input::CopyAs => self.do_copy(false, Some(text)),
                }
                self.refresh_all();
            }
            Dialog::Info(_) => {}
            Dialog::Pack { name, fmt, pw, rows, pane, .. } => {
                let dir = self.panes[pane].disk_dir().map(|p| p.to_path_buf());
                let o = self.other();
                let Some(dd) = self.panes[o].disk_dir().map(|p| p.to_path_buf()).or(dir) else {
                    return self.say(self.t("Destination invalide.", "Invalid destination.").to_string());
                };
                let name = archive::with_format_ext(&name, archive::FORMATS[fmt].1);
                let en = self.en();
                // Source : fichiers disque, ou entrées d'archive à extraire d'abord (dans la tâche).
                let (arc_src, paths): (Option<(Arc<archive::Archive>, String, Vec<String>, PathBuf)>, Vec<PathBuf>) = if let Loc::Arc { arch, inner, .. } = self.panes[pane].loc.clone() {
                    self.tmp_n += 1;
                    let d = self.tmp.join(self.tmp_n.to_string());
                    let names: Vec<String> = rows.iter().map(|r| r.full.clone()).collect();
                    let paths = rows.iter().map(|r| d.join(&r.name)).collect();
                    (Some((arch, inner, names, d)), paths)
                } else {
                    (None, rows.iter().filter_map(|r| self.panes[pane].disk_path(r)).collect())
                };
                self.pw_retry.insert("pack", Retry::Button("pack".into()));
                let label = format!("{} {name}", self.t("Compression", "Compressing"));
                self.jobs.spawn("pack", label, move |job| {
                    if let Some((arch, inner, names, tmp)) = arc_src {
                        let pred = |n: &str| names.iter().any(|x| n == x || n.starts_with(&format!("{x}/")));
                        job.set_detail(if en { "Extracting source…" } else { "Extraction de la source…" });
                        job.reset(arch.size_of(&pred).max(1));
                        arch.extract_p(&pred, &tmp, &inner, &|b| job.step(b))?;
                    }
                    job.set_detail(&name);
                    job.reset(paths.iter().map(|p| fsops::tree_size(p)).sum::<u64>().max(1));
                    let out = dd.join(&name);
                    archive::create_p(&out, &paths, Some(pw.as_str()), &|b| job.step(b))?;
                    Ok(format!("{} {name}{}", if en { "Archive created:" } else { "Archive créée :" }, if pw.is_empty() { "" } else { " 🔒" }))
                });
            }
            Dialog::Password { path, pw, .. } => {
                archive::set_password(&path, &pw);
                match self.retry.take() {
                    Some(Retry::Open(i)) => self.open_row(i),
                    Some(Retry::Button(id)) => {
                        let ctx = self.ctx_cache.clone();
                        if let Some(c) = ctx {
                            self.do_button(&id, &c);
                        }
                    }
                    None => {}
                }
            }
        }
    }

    fn ask_input(&mut self, kind: Input, title: &str, text: String) {
        let a = self.active;
        let rows = self.panes[a].targets();
        self.dialog = Some(Dialog::Input { kind, title: title.to_string(), text, rows, pane: a });
    }

    fn do_button(&mut self, id: &str, ctx: &egui::Context) {
        self.retry = Some(Retry::Button(id.to_string()));
        let a = self.active;
        match id {
            "drives" => self.panes[a].goto(Loc::Drives),
            "parent" => self.panes[a].parent(),
            "invert" => {
                let p = &mut self.panes[a];
                let all: Vec<String> = p.rows.iter().filter(|r| !r.is_parent).map(|r| r.full.clone()).collect();
                for n in all {
                    if !p.sel.remove(&n) {
                        p.sel.insert(n);
                    }
                }
            }
            "search" => self.open_search(),
            // Module IA / tri automatique : application séparée, lancée avec le dossier courant.
            "autoforge" => {
                let dir: Vec<PathBuf> = self.panes[a].disk_dir().map(|d| vec![d.to_path_buf()]).unwrap_or_default();
                if let Err(e) = launch("autoforge", &dir) {
                    self.say(e);
                }
            }
            "batchren" => self.open_rename(),
            "clear" => self.panes[a].sel.clear(),
            "all" => {
                let p = &mut self.panes[a];
                p.sel = p.rows.iter().filter(|r| !r.is_parent).map(|r| r.full.clone()).collect();
            }
            "copy" => self.do_copy(false, None),
            "copyas" => {
                if let Some(r) = self.panes[a].targets().first() {
                    let n = r.name.clone();
                    self.ask_input(Input::CopyAs, self.t("Copier sous le nom :", "Copy as:").to_string().as_str(), n);
                }
            }
            "move" => self.do_copy(true, None),
            "delete" => {
                let rows = self.panes[a].targets();
                if !rows.is_empty() && self.panes[a].disk_dir().is_some() {
                    let text = format!("{} {} {} ?", self.t("Supprimer définitivement", "Permanently delete"), rows.len(), self.t("élément(s)", "item(s)"));
                    self.dialog = Some(Dialog::Confirm { text, rows, pane: a });
                }
            }
            "mkdir" => self.ask_input(Input::MakeDir, self.t("Nom du nouveau dossier :", "New folder name:").to_string().as_str(), String::new()),
            "rename" => {
                if let Some(r) = self.panes[a].targets().first() {
                    let n = r.name.clone();
                    self.ask_input(Input::Rename, self.t("Nouveau nom :", "New name:").to_string().as_str(), n);
                }
            }
            "asc" => self.view(false),
            "hex" => self.view(true),
            "pic" => self.send_to("mediaforge-player", |p| matches!(kind_of(p), Some(MediaKind::Image)) || is_doc(p) || is_mesh(p)),
            "play" => self.send_to("mediaforge-player", |p| matches!(kind_of(p), Some(MediaKind::Audio | MediaKind::Video))),
            "convert" => self.send_to("mediaforge", |p| kind_of(p).is_some() && !is_doc(p)),
            "pack" => {
                let rows = self.panes[a].targets();
                if !rows.is_empty() {
                    let base = if rows.len() == 1 { rows[0].name.clone() } else { "archive".into() };
                    self.dialog = Some(Dialog::Pack { name: format!("{base}.zip"), fmt: 0, pw: String::new(), pw2: String::new(), show: false, rows, pane: a });
                }
            }
            "unpack" => self.unpack(),
            "diskinfo" => {
                let t = match self.panes[a].space() {
                    Some((u, t)) => format!("{}\n{} : {}\n{} : {}\n{} : {}", self.panes[a].title(), self.t("Total", "Total"), fsops::spaced(t), self.t("Utilisé", "Used"), fsops::spaced(u), self.t("Libre", "Free"), fsops::spaced(t - u)),
                    None => self.t("Information indisponible.", "No information available.").to_string(),
                };
                self.dialog = Some(Dialog::Info(t));
            }
            "disksize" => {
                if self.jobs.running("disksize") {
                    return self.busy();
                }
                let p = &self.panes[a];
                let paths: Vec<PathBuf> = p.rows.iter().filter(|r| r.is_dir && !r.is_parent && (p.sel.contains(&r.full) || p.sel.is_empty())).filter_map(|r| p.disk_path(r)).collect();
                let en = self.en();
                let label = self.t("Calcul de la taille", "Computing size").to_string();
                self.jobs.spawn("disksize", label, move |job| {
                    job.set_total(paths.len() as u64);
                    let (mut b, mut f, mut d) = (0, 0, 0);
                    for pp in &paths {
                        job.set_detail(pp.file_name().map(|x| x.to_string_lossy().to_string()).unwrap_or_default());
                        let s = fsops::dir_size(pp);
                        b += s.0;
                        f += s.1;
                        d += s.2 + 1;
                        if !job.step(1) {
                            return Err("Annulé".into());
                        }
                    }
                    Ok(format!("{} ({} {}) / {} {} / {} {}", fsops::mo(b, en), fsops::spaced(b), if en { "bytes" } else { "octets" }, f, if en { "files" } else { "fichiers" }, d, if en { "folders" } else { "dossiers" }))
                });
            }
            "execute" => {
                if let Some(r) = self.panes[a].targets().first().cloned() {
                    if let Ok(p) = self.materialize(a, &[r]) {
                        open_os(&p[0]);
                    }
                }
            }
            "edit" => {
                if let Some(r) = self.panes[a].targets().into_iter().find(|r| !r.is_dir) {
                    if let Ok(p) = self.materialize(a, &[r]) {
                        edit_os(&p[0]);
                    }
                }
            }
            "settings" => self.show_settings = true,
            "s2d" => {
                let l = self.panes[a].loc.clone();
                let o = self.other();
                self.panes[o].goto(l);
            }
            "swap" => self.panes.swap(0, 1),
            "quit" => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            _ => {}
        }
    }

    fn keys(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() || self.dialog.is_some() {
            return;
        }
        use egui::Key;
        let p = |k: Key| ctx.input(|i| i.key_pressed(k));
        let a = self.active;
        let n = self.panes[a].rows.len();
        if p(Key::Tab) {
            self.active = 1 - a;
        }
        if n > 0 {
            let c = self.panes[a].cursor.unwrap_or(0);
            let set = |v: usize, s: &mut Pane| {
                s.cursor = Some(v.min(n - 1));
                s.scroll_to_cursor = true;
            };
            if p(Key::ArrowDown) {
                set(c + 1, &mut self.panes[a]);
            }
            if p(Key::ArrowUp) {
                set(c.saturating_sub(1), &mut self.panes[a]);
            }
            if p(Key::PageDown) {
                set(c + 15, &mut self.panes[a]);
            }
            if p(Key::PageUp) {
                set(c.saturating_sub(15), &mut self.panes[a]);
            }
            if p(Key::Home) {
                set(0, &mut self.panes[a]);
            }
            if p(Key::End) {
                set(n - 1, &mut self.panes[a]);
            }
            if p(Key::Space) {
                let pane = &mut self.panes[a];
                if let Some(r) = pane.rows.get(c).filter(|r| !r.is_parent).cloned() {
                    if !pane.sel.remove(&r.full) {
                        pane.sel.insert(r.full);
                    }
                }
                set(c + 1, &mut self.panes[a]);
            }
            if p(Key::Enter) {
                self.open_row(c);
            }
        }
        // Frappe de lettres : placement sur le premier élément dont le nom commence par ce qui est tapé.
        let typed: String = ctx.input(|i| {
            if i.modifiers.command || i.modifiers.alt {
                return String::new();
            }
            i.events.iter().filter_map(|e| if let egui::Event::Text(t) = e { Some(t.as_str()) } else { None }).collect::<String>()
        });
        // Repli : si le système n'émet pas d'évènement texte (certains claviers / méthodes de saisie), on lit les touches.
        let typed = if typed.is_empty() {
            ctx.input(|i| {
                if i.modifiers.command || i.modifiers.alt || i.events.iter().any(|e| matches!(e, egui::Event::Text(_))) {
                    return String::new();
                }
                i.events.iter().filter_map(|e| match e {
                    egui::Event::Key { key, pressed: true, .. } => {
                        let n = key.name();
                        (n.chars().count() == 1 && n.chars().all(|c| c.is_ascii_alphanumeric())).then(|| n.to_string())
                    }
                    _ => None,
                }).collect::<String>()
            })
        } else {
            typed
        };
        let typed: String = typed.chars().filter(|c| !c.is_control() && !c.is_whitespace()).collect();
        if !typed.is_empty() && n > 0 {
            if self.typeahead.1.elapsed() > std::time::Duration::from_millis(1000) {
                self.typeahead.0.clear();
            }
            self.typeahead.0.push_str(&typed.to_lowercase());
            self.typeahead.1 = std::time::Instant::now();
            let q = self.typeahead.0.clone();
            let pane = &mut self.panes[a];
            if let Some(i) = pane.rows.iter().position(|r| !r.is_parent && r.name.to_lowercase().starts_with(&q)) {
                pane.cursor = Some(i);
                pane.scroll_to_cursor = true;
            }
        }
        if p(Key::F1) {
            ff_kit::help::trigger(ctx, "fileforge", "double-panneau");
        }
        if p(Key::Backspace) {
            self.panes[a].parent();
        }
        if p(Key::F5) {
            self.do_button("copy", ctx);
        }
        if p(Key::F6) {
            self.do_button("move", ctx);
        }
        if p(Key::F7) {
            self.do_button("mkdir", ctx);
        }
        if p(Key::F8) || p(Key::Delete) {
            self.do_button("delete", ctx);
        }
    }
}

/// Bouton de la colonne centrale.
/// Boutons de la barre centrale : (id, icône, libellé EN, libellé FR, couleur).
const BUTTONS: &[(&str, Icon, &str, &str, usize)] = &[
    ("drives", Icon::Folder, "Drives", "Lecteurs", 0),
    ("parent", Icon::Prev, "Parent", "Parent", 0),
    ("invert", Icon::Rotate, "Invert", "Inverser", 0),
    ("clear", Icon::Close, "Clear", "Aucune", 0),
    ("all", Icon::Check, "All", "Toutes", 0),
    ("copy", Icon::Copy, "Copy", "Copier", 1),
    ("copyas", Icon::Copy, "Copy As", "Copier sous", 1),
    ("move", Icon::Next, "Move", "Déplacer", 1),
    ("delete", Icon::Trash, "Delete", "Supprimer", 2),
    ("mkdir", Icon::Plus, "MakeDir", "Nv dossier", 3),
    ("rename", Icon::Pencil, "Rename", "Renommer", 3),
    ("asc", Icon::Doc, "Show Asc", "Voir texte", 4),
    ("hex", Icon::Sliders, "Show Hex", "Voir hexa", 4),
    ("pic", Icon::Picture, "Show Pic", "Voir image", 4),
    ("play", Icon::Play, "Play", "Lire", 4),
    ("convert", Icon::Tools, "Convert", "Convertir", 5),
    ("pack", Icon::Merge, "Pack", "Archiver", 5),
    ("unpack", Icon::Scissors, "Unpack", "Extraire", 5),
    ("diskinfo", Icon::Doc, "DiskInfo", "Info disque", 6),
    ("disksize", Icon::Sliders, "DiskSize", "Taille dossier", 6),
    ("search", Icon::ZoomIn, "Search", "Rechercher", 7),
    ("autoforge", Icon::Tools, "AutoForge", "AutoForge", 5),
    ("batchren", Icon::Sliders, "Batch Ren", "Renom. lot", 3),
    ("execute", Icon::Play, "Execute", "Exécuter", 7),
    ("edit", Icon::Pencil, "FileEdit", "Éditer", 7),
    ("settings", Icon::Gear, "Settings", "Réglages", 7),
];

/// Type de tâche de fond lancée par un bouton (pour le griser pendant qu'elle tourne).
fn job_key(id: &str) -> Option<&'static str> {
    Some(match id {
        "disksize" => "disksize",
        _ => return None,
    })
}

fn cbtn(ui: &mut egui::Ui, icon: Icon, text: &str, col: Color32, retro: bool, enabled: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 25.0), egui::Sense::click());
    let pal = ff_kit::theme::current();
    let p = ui.painter();
    if retro {
        let f = if resp.hovered() && enabled { pal.accent.gamma_multiply(0.35) } else { pal.card };
        p.rect_filled(rect, 0.0, f);
        p.rect_stroke(rect, 0.0, egui::Stroke::new(1.0, pal.text), egui::StrokeKind::Inside);
        p.text(rect.center(), egui::Align2::CENTER_CENTER, text, egui::FontId::monospace(14.0), if enabled { pal.text } else { pal.weak });
    } else {
        let f = if enabled { if resp.hovered() { col.gamma_multiply(1.15) } else { col } } else { col.gamma_multiply(0.35) };
        p.rect_filled(rect, 5.0, f);
        let fg = ff_kit::theme::on_color(col);
        icons::paint(p, egui::Rect::from_center_size(rect.left_center() + egui::vec2(15.0, 0.0), egui::vec2(15.0, 15.0)), icon, fg);
        p.text(rect.left_center() + egui::vec2(30.0, 0.0), egui::Align2::LEFT_CENTER, text, egui::FontId::proportional(13.5), fg);
    }
    resp.clicked() && enabled
}

fn row_icon(r: &Row) -> Icon {
    if r.is_dir {
        Icon::Folder
    } else if r.is_arc {
        Icon::Merge
    } else {
        let p = Path::new(&r.name);
        match kind_of(p) {
            Some(MediaKind::Video) => Icon::Film,
            Some(MediaKind::Audio) => Icon::Note,
            Some(MediaKind::Image) => Icon::Picture,
            None if is_doc(p) || is_mesh(p) => Icon::Picture,
            None => Icon::Doc,
        }
    }
}

impl App {
    /// Dessine un panneau ; retourne (ligne cliquée, double-clic).
    fn draw_pane(&mut self, ui: &mut egui::Ui, idx: usize) -> Option<(usize, bool)> {
        let pal = ff_kit::theme::current();
        let retro = self.retro();
        let font = if retro { egui::FontId::monospace(14.0) } else { egui::FontId::proportional(14.0) };
        let active = self.active == idx;
        let mut out = None;
        // En-tête : espace disque.
        let hdr = match self.panes[idx].space() {
            Some((u, t)) => format!("Used {}  Total {}  {}%", pane::human(u), pane::human(t), if t > 0 { u * 100 / t } else { 0 }),
            None => self.panes[idx].title(),
        };
        let (hr, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 24.0), egui::Sense::hover());
        let hc = pal.header.unwrap_or(pal.panel);
        ui.painter().rect_filled(hr, if retro { 0.0 } else { 4.0 }, hc);
        ui.painter().text(hr.center(), egui::Align2::CENTER_CENTER, hdr, font.clone(), if pal.header.is_some() { ff_kit::theme::on_color(hc) } else { pal.text });

        let list_h = ui.available_height() - 30.0;
        let w = ui.available_width();
        let (frame_rect, frame_resp) = ui.allocate_exact_size(egui::vec2(w, list_h.max(60.0)), egui::Sense::click());
        let mut sort_change: Option<(u8, bool)> = None;
        let (sk, sd) = (self.panes[idx].sort_key, self.panes[idx].sort_desc);
        let en = self.en();
        frame_resp.context_menu(|ui| sort_menu(ui, sk, sd, en, &mut sort_change));
        ui.painter().rect_stroke(frame_rect, 0.0, egui::Stroke::new(if active { 2.0 } else { 1.0 }, if active { pal.accent } else { pal.stroke }), egui::StrokeKind::Inside);
        let inner = frame_rect.shrink(2.0);
        ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
            egui::ScrollArea::vertical().id_salt(("list", idx)).auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let stick = self.panes[idx].scroll_to_cursor;
                for i in 0..self.panes[idx].rows.len() {
                    let r = self.panes[idx].rows[i].clone();
                    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 20.0), egui::Sense::click_and_drag());
                    let sel = self.panes[idx].sel.contains(&r.full);
                    let cur = active && self.panes[idx].cursor == Some(i);
                    let mut fg = if r.is_dir { pal.accent } else if r.is_arc { pal.ok } else { pal.text };
                    if sel {
                        ui.painter().rect_filled(rect, 0.0, pal.accent);
                        fg = ff_kit::theme::on_color(pal.accent);
                    } else if resp.hovered() {
                        ui.painter().rect_filled(rect, 0.0, pal.accent.gamma_multiply(0.15));
                    }
                    if cur {
                        ui.painter().rect_stroke(rect, 0.0, egui::Stroke::new(1.0, pal.text), egui::StrokeKind::Inside);
                        if stick {
                            resp.scroll_to_me(None);
                        }
                    }
                    let ic = egui::Rect::from_center_size(rect.left_center() + egui::vec2(12.0, 0.0), egui::vec2(14.0, 14.0));
                    if !r.is_parent {
                        icons::paint(ui.painter(), ic, row_icon(&r), fg);
                    }
                    // Colonne taille (à droite) ; le nom dispose de la place restante et défile lentement s'il est trop long.
                    let size_txt = if !r.is_dir { fsops::spaced(r.size) } else if !r.is_parent { "<dir>".to_string() } else { String::new() };
                    let size_col = if r.is_dir && !sel { pal.weak } else { fg };
                    let size_w = if size_txt.is_empty() { 0.0 } else { ui.painter().layout_no_wrap(size_txt.clone(), font.clone(), size_col).size().x };
                    if !size_txt.is_empty() {
                        ui.painter().text(rect.right_center() - egui::vec2(8.0, 0.0), egui::Align2::RIGHT_CENTER, &size_txt, font.clone(), size_col);
                    }
                    let name_rect = egui::Rect::from_min_max(rect.left_top() + egui::vec2(26.0, 0.0), egui::pos2(rect.right() - size_w - 18.0, rect.bottom()));
                    let full_g = ui.painter().layout_no_wrap(r.name.clone(), font.clone(), fg);
                    let overflow = full_g.size().x - name_rect.width();
                    if overflow > 0.0 && (resp.hovered() || cur) {
                        // Va-et-vient lent (30 px/s) avec pause aux extrémités.
                        let t = ui.input(|i| i.time) as f32;
                        let (speed, pause) = (30.0f32, 1.0f32);
                        let travel = overflow / speed;
                        let cyc = 2.0 * (travel + pause);
                        let m = t % cyc;
                        let off = if m < pause { 0.0 } else if m < pause + travel { (m - pause) * speed } else if m < 2.0 * pause + travel { overflow } else { overflow - (m - 2.0 * pause - travel) * speed };
                        let pos = egui::pos2(name_rect.left() - off, name_rect.center().y - full_g.size().y / 2.0);
                        ui.painter().with_clip_rect(name_rect).galley(pos, full_g, fg);
                        ui.ctx().request_repaint();
                    } else if overflow > 0.0 {
                        let job = egui::text::LayoutJob::simple_singleline(r.name.clone(), font.clone(), fg);
                        let mut job = job;
                        job.wrap = egui::text::TextWrapping { max_width: name_rect.width(), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
                        let g = ui.painter().layout_job(job);
                        ui.painter().galley(egui::pos2(name_rect.left(), name_rect.center().y - g.size().y / 2.0), g, fg);
                    } else {
                        ui.painter().galley(egui::pos2(name_rect.left(), name_rect.center().y - full_g.size().y / 2.0), full_g, fg);
                    }
                    resp.context_menu(|ui| sort_menu(ui, sk, sd, en, &mut sort_change));
                    if resp.drag_started() && !r.is_parent && self.drag.is_none() {
                        // Démarre un glisser : la sélection si la ligne en fait partie, sinon la ligne seule.
                        let p = &self.panes[idx];
                        let rows: Vec<Row> = if p.sel.contains(&r.full) { p.targets() } else { vec![r.clone()] };
                        let paths: Vec<PathBuf> = rows.iter().filter_map(|x| p.disk_path(x)).collect();
                        let n = rows.len();
                        self.drag = Some((idx, paths, n));
                    }
                    if resp.double_clicked() {
                        out = Some((i, true));
                    } else if resp.clicked() {
                        out = Some((i, false));
                    }
                }
                self.panes[idx].scroll_to_cursor = false;
            });
        });
        if let Some((k, d)) = sort_change {
            self.panes[idx].sort_key = k;
            self.panes[idx].sort_desc = d;
            self.panes[idx].resort();
        }
        // Barre de chemin éditable.
        let mut go: Option<PathBuf> = None;
        let focused = {
            let pane = &mut self.panes[idx];
            let resp = ui.add(egui::TextEdit::singleline(&mut pane.path_edit).font(font.clone()).desired_width(f32::INFINITY));
            if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                go = Some(PathBuf::from(pane.path_edit.trim()));
            }
            resp.has_focus()
        };
        if let Some(p) = go {
            if p.is_dir() {
                self.panes[idx].goto(Loc::Disk(p));
            } else if p.is_file() && archive::kind_of(&p).is_some() {
                match archive::Archive::open(&p) {
                    Ok(a) => {
                        let back = p.parent().map(|x| x.to_path_buf()).unwrap_or_default();
                        self.panes[idx].goto(Loc::Arc { arch: std::sync::Arc::new(a), inner: String::new(), back });
                    }
                    Err(e) => self.say(e),
                }
            } else {
                self.say(self.t("Chemin introuvable.", "Path not found.").to_string());
                self.panes[idx].path_edit = self.panes[idx].title();
            }
        } else if !focused {
            let t = self.panes[idx].title();
            if self.panes[idx].path_edit != t && !ui.ctx().egui_wants_keyboard_input() {
                self.panes[idx].path_edit = t;
            }
        }
        if let Some(e) = self.panes[idx].err.clone() {
            ui.colored_label(pal.err, e);
        }
        out
    }

    fn buttons(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let retro = self.retro();
        let en = self.en();
        let g = |fr: &'static str, en_: &'static str| if en { en_ } else { fr };
        let (nav, cp, del, yel, view, arc, inf, gen) = (
            Color32::from_rgb(0x25, 0x41, 0x93),
            Color32::from_rgb(0x00, 0x97, 0x3a),
            Color32::from_rgb(0xe3, 0x05, 0x13),
            Color32::from_rgb(0xd9, 0xa8, 0x00),
            Color32::from_rgb(0x00, 0x9e, 0xe3),
            Color32::from_rgb(0x82, 0x35, 0x8b),
            Color32::from_rgb(0x5b, 0x6b, 0x7f),
            Color32::from_rgb(0x44, 0x4c, 0x5c),
        );
        let colors = [nav, cp, del, yel, view, arc, inf, gen];
        let hidden = self.settings.hidden_buttons.clone();
        let mut clicked: Option<&str> = None;
        egui::ScrollArea::vertical().id_salt("btns").auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            for (id, ic, e, f, ci) in BUTTONS {
                if hidden.iter().any(|h| h == id) {
                    continue;
                }
                // Tâche du même type en cours : bouton grisé pour éviter un double lancement.
                let busy = job_key(id).map(|k| self.jobs.running(k)).unwrap_or(false);
                let txt = if busy { format!("{}…", g(f, e)) } else { g(f, e).to_string() };
                if cbtn(ui, *ic, &txt, colors[*ci], retro, !busy) {
                    clicked = Some(id);
                }
            }
        });
        if let Some(id) = clicked {
            self.do_button(id, ctx);
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        if let Some(d) = self.dialog.take() {
            let mut keep = true;
            let mut run = false;
            let d = match d {
                Dialog::Confirm { text, rows, pane } => {
                    egui::Window::new(self.t("Confirmation", "Confirm")).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                        ui.label(&text);
                        ui.horizontal(|ui| {
                            if ui.add_enabled(true, egui::Button::new(RichText::new(self.t("Supprimer", "Delete")).color(ff_kit::theme::current().err))).clicked() {
                                run = true;
                            }
                            if ui.button(self.t("Annuler", "Cancel")).clicked() {
                                keep = false;
                            }
                        });
                    });
                    Dialog::Confirm { text, rows, pane }
                }
                Dialog::Input { kind, title, mut text, rows, pane } => {
                    egui::Window::new(&title).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                        let r = ui.add(egui::TextEdit::singleline(&mut text).desired_width(320.0));
                        r.request_focus();
                        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        ui.horizontal(|ui| {
                            if ui.button("OK").clicked() || enter {
                                run = true;
                            }
                            if ui.button(self.t("Annuler", "Cancel")).clicked() {
                                keep = false;
                            }
                        });
                    });
                    Dialog::Input { kind, title, text, rows, pane }
                }
                Dialog::Pack { mut name, mut fmt, mut pw, mut pw2, mut show, rows, pane } => {
                    egui::Window::new(self.t("Créer une archive", "Create archive")).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                        ff_kit::help::button(ui, "fileforge", "archives", "?");
                        let rar_ok = archive::rar_exe().is_some();
                        ui.label(self.t("Type de compression :", "Compression type:"));
                        ui.horizontal_wrapped(|ui| {
                            for (i, (label, ext, _)) in archive::FORMATS.iter().enumerate() {
                                let ok = *ext != ".rar" || rar_ok;
                                if ui.add_enabled(ok, egui::RadioButton::new(fmt == i, *label)).clicked() {
                                    fmt = i;
                                    name = archive::with_format_ext(&name, ext);
                                }
                            }
                        });
                        if !rar_ok {
                            ui.label(RichText::new(self.t("RAR : nécessite WinRAR (rar.exe) installé.", "RAR: requires WinRAR (rar.exe) installed.")).small().weak());
                        }
                        ui.add_space(4.0);
                        ui.label(self.t("Nom de l'archive :", "Archive name:"));
                        let r = ui.add(egui::TextEdit::singleline(&mut name).desired_width(340.0));
                        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        let can_pw = archive::FORMATS[fmt].2;
                        ui.add_space(4.0);
                        ui.add_enabled_ui(can_pw, |ui| {
                            ui.label(self.t("Mot de passe (facultatif) :", "Password (optional):"));
                            ui.add(egui::TextEdit::singleline(&mut pw).password(!show).desired_width(340.0));
                            ui.add(egui::TextEdit::singleline(&mut pw2).password(!show).desired_width(340.0).hint_text(self.t("confirmation", "confirm")));
                            ui.checkbox(&mut show, self.t("Afficher", "Show"));
                        });
                        if !can_pw {
                            pw.clear();
                            pw2.clear();
                            ui.label(RichText::new(self.t("Ce format ne gère pas le mot de passe (utilisez ZIP, 7Z ou RAR).", "This format has no password support (use ZIP, 7Z or RAR).")).small().weak());
                        } else if fmt == 0 && !pw.is_empty() {
                            ui.label(RichText::new(self.t("ZIP chiffré en AES-256 (7-Zip, WinRAR, FileForge ; pas l'Explorateur Windows).", "ZIP encrypted with AES-256 (7-Zip, WinRAR, FileForge; not Windows Explorer).")).small().weak());
                        }
                        let mismatch = can_pw && pw != pw2;
                        if mismatch {
                            ui.colored_label(ff_kit::theme::current().err, self.t("Les mots de passe diffèrent.", "Passwords do not match."));
                        }
                        ui.horizontal(|ui| {
                            if (ui.add_enabled(!mismatch && !name.trim().is_empty() , egui::Button::new("OK")).clicked() || enter) && !mismatch {
                                run = true;
                            }
                            if ui.button(self.t("Annuler", "Cancel")).clicked() {
                                keep = false;
                            }
                        });
                    });
                    Dialog::Pack { name, fmt, pw, pw2, show, rows, pane }
                }
                Dialog::Password { path, mut pw, bad } => {
                    egui::Window::new(self.t("Archive protégée", "Protected archive")).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                        ui.label(path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
                        if bad {
                            ui.colored_label(ff_kit::theme::current().err, self.t("Mot de passe incorrect.", "Wrong password."));
                        }
                        let r = ui.add(egui::TextEdit::singleline(&mut pw).password(true).desired_width(300.0).hint_text(self.t("mot de passe", "password")));
                        r.request_focus();
                        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        ui.horizontal(|ui| {
                            if ui.button("OK").clicked() || enter {
                                run = true;
                            }
                            if ui.button(self.t("Annuler", "Cancel")).clicked() {
                                keep = false;
                            }
                        });
                    });
                    Dialog::Password { path, pw, bad }
                }
                Dialog::Info(t) => {
                    egui::Window::new("Info").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                        ui.label(&t);
                        if ui.button("OK").clicked() {
                            keep = false;
                        }
                    });
                    Dialog::Info(t)
                }
            };
            if run {
                self.run_dialog(d);
            } else if keep {
                self.dialog = Some(d);
            }
        }
        if let Some(v) = &self.viewer {
            let mut open = true;
            egui::Window::new(format!("{} {}", if v.hex { "Hex" } else { "Asc" }, v.title)).open(&mut open).default_size([640.0, 420.0]).show(ctx, |ui| {
                egui::ScrollArea::both().show(ui, |ui| {
                    ui.add(egui::Label::new(RichText::new(&v.text).monospace()).extend());
                });
            });
            if !open {
                self.viewer = None;
            }
        }
        if self.show_settings {
            let mut open = true;
            let mut chg = false;
            egui::Window::new(self.t("Réglages", "Settings")).open(&mut open).collapsible(false).resizable(true).scroll([false, true]).default_size([440.0, 520.0]).show(ctx, |ui| {
                ff_kit::help::links(ui, "fileforge", &[("Réglages", "reglages"), ("Menu contextuel", "menu-contextuel"), ("Boutons", "boutons")]);
                ui.horizontal(|ui| {
                    ui.label(self.t("Langue :", "Language:"));
                    for (g, n) in [(Lang::Fr, "Français"), (Lang::En, "English")] {
                        if ui.selectable_label(self.settings.lang == g, n).clicked() {
                            self.settings.lang = g;
                            for p in &mut self.panes { p.en = g == Lang::En; }
                            chg = true;
                        }
                    }
                });
                ui.horizontal(|ui| {
                    if icons::button(ui, Icon::Sun, 18.0, !self.settings.dark, self.t("Clair", "Light")).clicked() && self.settings.dark {
                        self.settings.dark = false;
                        chg = true;
                    }
                    if icons::button(ui, Icon::Moon, 18.0, self.settings.dark, self.t("Sombre", "Dark")).clicked() && !self.settings.dark {
                        self.settings.dark = true;
                        chg = true;
                    }
                });
                let lbl = self.t("Navigation en simple clic (Ctrl+clic = sélection)", "Single-click navigation (Ctrl+click = select)").to_string();
                if ui.checkbox(&mut self.settings.single_click, lbl).changed() {
                    chg = true;
                }
                let lbl = self.t("Fermer cette fenêtre en cliquant à l'extérieur", "Close this window by clicking outside").to_string();
                if ui.checkbox(&mut self.settings.close_on_outside, lbl).changed() {
                    chg = true;
                }
                if shell::SUPPORTED {
                    ui.separator();
                    ui.label(RichText::new(self.t("Explorateur Windows", "Windows Explorer")).strong());
                    ui.horizontal(|ui| {
                        let exe = std::env::current_exe().unwrap_or_default();
                        if ui.button(self.t("Ajouter « Gérer avec FileForge »", "Add “Manage with FileForge”")).clicked() {
                            let r = shell::install(&exe, self.t("Gérer avec FileForge", "Manage with FileForge"));
                            self.msg = match r { Ok(_) => self.t("Menu contextuel ajouté.", "Context menu added.").to_string(), Err(e) => e.to_string() };
                        }
                        if ui.button(self.t("Retirer", "Remove")).clicked() {
                            let r = shell::uninstall();
                            self.msg = match r { Ok(_) => self.t("Menu contextuel retiré.", "Context menu removed.").to_string(), Err(e) => e.to_string() };
                        }
                    });
                    ui.separator();
                }
                let title_b = self.t("Boutons de la barre centrale", "Middle bar buttons").to_string();
                egui::CollapsingHeader::new(title_b).default_open(false).show(ui, |ui| {
                    let en = self.en();
                    ui.horizontal(|ui| {
                        if ui.small_button(if en { "Show all" } else { "Tout afficher" }).clicked() {
                            self.settings.hidden_buttons.clear();
                            chg = true;
                        }
                    });
                    ui.horizontal_wrapped(|ui| {
                        for (id, _, e, f, _) in BUTTONS {
                            let mut on = !self.settings.hidden_buttons.iter().any(|h| h == id);
                            if ui.checkbox(&mut on, if en { *e } else { *f }).changed() {
                                self.settings.hidden_buttons.retain(|h| h != id);
                                if !on {
                                    self.settings.hidden_buttons.push(id.to_string());
                                }
                                chg = true;
                            }
                        }
                    });
                });
                let title = self.t("Dossiers affichés dans « Lecteurs »", "Folders shown in \"Drives\"").to_string();
                egui::CollapsingHeader::new(title).default_open(true).show(ui, |ui| {
                    let en = self.en();
                    for (id, fr, e, p) in pane::standard_places() {
                        if p.is_none() {
                            continue;
                        }
                        let mut on = !self.settings.hidden_places.iter().any(|h| h == id);
                        if ui.checkbox(&mut on, if en { e } else { fr }).changed() {
                            self.settings.hidden_places.retain(|h| h != id);
                            if !on {
                                self.settings.hidden_places.push(id.to_string());
                            }
                            chg = true;
                        }
                    }
                    let mut rm: Option<usize> = None;
                    for (i, c) in self.settings.custom_places.iter().enumerate() {
                        ui.horizontal(|ui| {
                            if ui.small_button(if self.settings.lang == Lang::En { "Remove" } else { "Retirer" }).clicked() {
                                rm = Some(i);
                            }
                            ui.label(c);
                        });
                    }
                    if let Some(i) = rm {
                        self.settings.custom_places.remove(i);
                        chg = true;
                    }
                    if ui.button(if en { "Add a folder…" } else { "Ajouter un dossier…" }).clicked() {
                        if let Some(d) = rfd::FileDialog::new().pick_folder() {
                            let s = d.display().to_string();
                            if !self.settings.custom_places.contains(&s) {
                                self.settings.custom_places.push(s);
                                chg = true;
                            }
                        }
                    }
                });
                let mut id = self.settings.theme.clone();
                if ff_kit::theme::style_picker(ui, &mut id, self.settings.dark, self.en()) {
                    self.settings.theme = id;
                    chg = true;
                }
                ui.separator();
                ui.label(self.t("Raccourcis : Tab = autre panneau · Espace = sélection · Entrée = ouvrir · Retour = parent · F5 copier · F6 déplacer · F7 dossier · F8/Suppr supprimer", "Shortcuts: Tab = other pane · Space = select · Enter = open · Backspace = parent · F5 copy · F6 move · F7 folder · F8/Del delete"));
                ui.label(self.t("Archives : ZIP, 7z, TAR(.GZ), LHA/LZH ; RAR/CAB/ISO via 7-Zip.", "Archives: ZIP, 7z, TAR(.GZ), LHA/LZH; RAR/CAB/ISO via 7-Zip."));
                ui.label(RichText::new(format!("FileForge v{VERSION}")).small().weak());
            });
            if chg {
                let mut s = ff_profiles::load();
                s.theme = self.settings.theme.clone();
                s.dark = self.settings.dark;
                s.lang = self.settings.lang;
                s.single_click = self.settings.single_click;
                s.hidden_places = self.settings.hidden_places.clone();
                s.custom_places = self.settings.custom_places.clone();
                s.hidden_buttons = self.settings.hidden_buttons.clone();
                s.close_on_outside = self.settings.close_on_outside;
                let _ = ff_profiles::save(&s);
                ff_kit::theme::apply(ctx, ff_kit::theme::Style::from_id(&self.settings.theme), self.settings.dark);
                self.sync_places();
            }
            // Clic en dehors de la fenêtre : fermeture (option).
            if self.settings.close_on_outside && self.outside.check(ctx, open) {
                open = false;
            }
            if !open {
                self.show_settings = false;
            }
        } else {
            self.outside.check(ctx, false);
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, root: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = root.ctx().clone();
        // Thème partagé avec MediaForge / Player (relu toutes les 2 s).
        if self.theme_check.elapsed().as_secs() >= 2 {
            self.theme_check = std::time::Instant::now();
            let sh = ff_profiles::load();
            if sh.theme != self.settings.theme || sh.dark != self.settings.dark {
                self.settings.theme = sh.theme;
                self.settings.dark = sh.dark;
                ff_kit::theme::apply(&ctx, ff_kit::theme::Style::from_id(&self.settings.theme), self.settings.dark);
            }
            ctx.request_repaint_after(std::time::Duration::from_secs(2));
        }
        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::F)) {
            self.open_search();
        }
        self.ctx_cache = Some(ctx.clone());
        // Tâches de fond terminées : résultat + rafraîchissement des panneaux.
        for (job, res) in self.jobs.poll() {
            match res {
                Ok(m) => self.say(m),
                Err(e) if archive::is_pw_error(&e) => self.retry = self.pw_retry.remove(job.key),
                Err(e) => self.say(e),
            }
            self.refresh_all();
        }
        self.rename_apply_pending();
        if self.jobs.any() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        // Archive chiffrée : demande du mot de passe (l'action est relancée après saisie).
        if let Some((path, bad)) = archive::take_pw_request() {
            if self.dialog.is_none() {
                self.dialog = Some(Dialog::Password { path, pw: String::new(), bad });
            }
        }
        for p in &mut self.panes {
            if p.err.as_deref().map(archive::is_pw_error).unwrap_or(false) {
                p.err = None;
            }
        }
        self.keys(&ctx);
        let pal = ff_kit::theme::current();
        let retro = self.retro();
        let font = if retro { egui::FontId::monospace(13.0) } else { egui::FontId::proportional(13.0) };

        // Barre du bas : actions de session.
        egui::Panel::bottom("bottom").show(root, |ui| {
            ui.horizontal(|ui| {
                let a = self.active;
                let info = self.panes[a].cursor.and_then(|i| self.panes[a].rows.get(i)).filter(|r| !r.is_parent).map(|r| format!("{}   {}   {}", r.name, fsops::spaced(r.size), fmt_time(r.mtime))).unwrap_or_default();
                ui.label(RichText::new(info).font(font.clone()));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    for (id, fr, en) in [("quit", "Quitter", "Quit"), ("settings", "Réglages", "Settings"), ("swap", "Échanger", "Swap"), ("s2d", "S→D", "S→D")] {
                        if ui.button(self.t(fr, en)).clicked() {
                            self.do_button(id, &ctx);
                        }
                    }
                });
            });
        });
        // Zone d'état (sélection) comme FileMaster.
        ff_kit::help::show_error(&ctx);
        egui::Panel::bottom("status").show(root, |ui| {
            let (fs, f, ds, d, bs, b) = self.panes[self.active].stats();
            let txt = format!("{}/{}/{} files {}/{}/{} dirs {}/{}/{} bytes", fs, fs, f, ds, ds, d, fsops::spaced(bs), fsops::spaced(bs), fsops::spaced(b));
            ui.vertical_centered(|ui| {
                ui.label(RichText::new(txt).font(font.clone()).strong());
                ui.label(RichText::new(self.msg.clone()).font(font.clone()).color(pal.weak));
            });
            // Tâches : la tâche en cours avec progression ; les autres sont dans la file (fenêtre).
            let mut cancel: Option<Arc<jobs::Job>> = None;
            let waiting = self.jobs.active().iter().filter(|j| j.pending()).count();
            for job in self.jobs.active().iter().filter(|j| !j.pending()) {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new(&job.label).strong());
                    match job.fraction() {
                        Some(f) => {
                            ui.add(egui::ProgressBar::new(f).desired_width(220.0).show_percentage());
                        }
                        None => {
                            ui.add(egui::ProgressBar::new(0.0).desired_width(220.0).animate(true));
                        }
                    }
                    let d = job.detail();
                    if !d.is_empty() {
                        ui.add(egui::Label::new(RichText::new(d).small().color(pal.weak)).truncate());
                    }
                    if ui.small_button(self.t("Annuler", "Cancel")).clicked() {
                        cancel = Some(job.clone());
                    }
                });
            }
            if waiting > 0 || !self.jobs.history.is_empty() || self.show_queue {
                ui.horizontal(|ui| {
                    let t = if waiting > 0 { format!("{} ({waiting})", self.t("File d'attente", "Queue")) } else { self.t("File d'attente", "Queue").to_string() };
                    if ui.small_button(t).clicked() {
                        self.show_queue = !self.show_queue;
                    }
                });
            }
            if let Some(j) = cancel {
                self.jobs.cancel(&j);
            }
        });
        self.queue_window(&ctx);

        egui::CentralPanel::default().show(root, |ui| {
            let full = ui.max_rect();
            let bw = 124.0;
            let pw = ((full.width() - bw - 16.0) / 2.0).max(120.0);
            let left = egui::Rect::from_min_size(full.min, egui::vec2(pw, full.height()));
            let mid = egui::Rect::from_min_size(egui::pos2(left.right() + 8.0, full.top()), egui::vec2(bw, full.height()));
            let right = egui::Rect::from_min_size(egui::pos2(mid.right() + 8.0, full.top()), egui::vec2(pw, full.height()));
            self.pane_rects = [left, right];
            // Un clic n'importe où dans un panneau (bandeau d'espace disque, zone vide…) lui donne le focus.
            if let Some(p) = ctx.input(|i| i.pointer.primary_clicked().then(|| i.pointer.interact_pos()).flatten()) {
                if let Some(i) = self.pane_at(p) {
                    self.active = i;
                }
            }
            let mut ev: [Option<(usize, bool)>; 2] = [None, None];
            for (i, r) in [(0usize, left), (1usize, right)] {
                ev[i] = ui.scope_builder(egui::UiBuilder::new().max_rect(r), |ui| self.draw_pane(ui, i)).inner;
            }
            ui.scope_builder(egui::UiBuilder::new().max_rect(mid), |ui| self.buttons(ui, &ctx));
            for (i, e) in ev.into_iter().enumerate() {
                if let Some((row, dbl)) = e {
                    self.active = i;
                    self.panes[i].cursor = Some(row);
                    let ctrl = ctx.input(|i| i.modifiers.command);
                    let one = self.settings.single_click && !ctrl;
                    if dbl {
                        if !self.settings.single_click {
                            self.open_row(row);
                        } else if self.panes[i].rows.get(row).map(|r| !r.is_dir && !r.is_arc).unwrap_or(false) {
                            self.open_row(row);
                        }
                    } else if one {
                        // Navigation en simple clic : dossiers, archives et « .. » s'ouvrent, les fichiers se contentent d'être pointés.
                        let disk = matches!(self.panes[i].loc, Loc::Disk(_));
                        if self.panes[i].rows.get(row).map(|r| r.is_dir || r.is_arc || (disk && !r.is_parent && ({ let p = Path::new(&r.full); kind_of(p).is_some() || is_doc(p) || is_mesh(p) }))).unwrap_or(false) {
                            // Dossiers/archives : navigation ; fichiers pris en charge par le lecteur : ouverture dans MediaForge Player.
                            self.open_row(row);
                        }
                    } else if let Some(r) = self.panes[i].rows.get(row).filter(|r| !r.is_parent).cloned() {
                        if !self.panes[i].sel.remove(&r.full) {
                            self.panes[i].sel.insert(r.full);
                        }
                    }
                }
            }
        });
        self.drag_and_drop(&ctx, frame);
        self.dialogs(&ctx);
        self.search_window(&ctx);
        self.rename_window(&ctx);
    }
}

impl Drop for App {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.tmp);
    }
}

fn main() -> eframe::Result<()> {
    let start = std::env::args_os().nth(1).map(PathBuf::from).filter(|p| p.is_dir() || archive::kind_of(p).is_some());
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title("FileForge").with_inner_size([1080.0, 680.0]).with_min_inner_size([760.0, 420.0]),
        ..Default::default()
    };
    eframe::run_native("FileForge", options, Box::new(move |cc| Ok(Box::new(App::new(cc, start)))))
}

/// Menu contextuel de tri : nom, date, type, taille, et sens.
fn sort_menu(ui: &mut egui::Ui, key: u8, desc: bool, en: bool, out: &mut Option<(u8, bool)>) {
    ui.label(egui::RichText::new(if en { "Sort by" } else { "Trier par" }).strong());
    for (k, fr, e) in [(0u8, "Nom", "Name"), (1, "Date", "Date"), (2, "Type", "Type"), (3, "Poids (taille)", "Size")] {
        if ui.radio(key == k, if en { e } else { fr }).clicked() {
            *out = Some((k, desc));
            ui.close();
        }
    }
    ui.separator();
    if ui.radio(!desc, if en { "Ascending" } else { "Croissant" }).clicked() {
        *out = Some((key, false));
        ui.close();
    }
    if ui.radio(desc, if en { "Descending" } else { "Décroissant" }).clicked() {
        *out = Some((key, true));
        ui.close();
    }
}
