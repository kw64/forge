//! Interface du lecteur : rendu libmpv (OpenGL) sous egui, commandes auto-masquées, liste de lecture. v0.4.0
use crate::derush::{self, Note};
use crate::doc;
use crate::i18n::tr;
use ff_kit::icons::{self, Icon};
use crate::playlist::{Playlist, Repeat};
use crate::store::{save_settings, PlayerSettings, Resume};
use eframe::egui::{self, Color32, Key, RichText};
use ff_core::media::{is_doc, is_mesh, natural_cmp, siblings, MediaKind};

/// Comme `ff_core::media::kind_of`, avec les documents (PDF/AI/PSD/EPS) comptés comme images.
fn kind_of(p: &Path) -> Option<MediaKind> {
    if ff_online::is_url(&p.to_string_lossy()) {
        return Some(MediaKind::Video);
    }
    ff_core::media::kind_of(p).or_else(|| (is_doc(p) || is_mesh(p)).then_some(MediaKind::Image))
}
use ff_core::Lang;
use ff_mpv::{EndReason, Event, Mpv, RenderContext};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn accent() -> Color32 {
    ff_kit::theme::current().accent
}

/// Visionneuse 3D (STL/OBJ) : rendu logiciel dans une texture egui.
struct MeshView {
    mesh: crate::mesh::Mesh,
    yaw: f32,
    pitch: f32,
    zoom: f32,
    wire: bool,
    tex: Option<egui::TextureHandle>,
    size: [usize; 2],
    dirty: bool,
}

const VU_FILTER: &str = "@vu:lavfi=[astats=metadata=1:reset=1]";

/// Extrait les niveaux crête (dB → 0..1 sur -60..0 dB) gauche/droite du JSON `af-metadata/vu` de mpv.
fn parse_vu(json: &str) -> Option<[f32; 2]> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let db = |k: &str| -> Option<f32> {
        let x: f32 = v.get(k)?.as_str()?.parse().ok()?;
        Some(if x.is_finite() { x } else { -90.0 })
    };
    let l = db("lavfi.astats.1.Peak_level").or_else(|| db("lavfi.astats.Overall.Peak_level"))?;
    let r = db("lavfi.astats.2.Peak_level").unwrap_or(l);
    let n = |d: f32| ((d + 60.0) / 60.0).clamp(0.0, 1.0);
    Some([n(l), n(r)])
}

#[cfg(test)]
mod vu_tests {
    use super::parse_vu;
    #[test]
    fn parses_levels() {
        let j = r#"{"lavfi.astats.1.Peak_level":"-30.0","lavfi.astats.2.Peak_level":"-inf"}"#;
        let v = parse_vu(j).unwrap();
        assert!((v[0] - 0.5).abs() < 1e-4 && v[1] == 0.0);
        assert!(parse_vu("{}").is_none());
    }
}
const HIDE_AFTER: Duration = Duration::from_millis(2500);
const SPEEDS: &[f64] = &[0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 2.0, 3.0, 4.0];

struct Track {
    id: i64,
    audio: bool,
    label: String,
    selected: bool,
}

pub struct App {
    l: Lang,
    set: PlayerSettings,
    resume: Resume,
    mpv: Option<Mpv>,
    render: Option<Arc<Mutex<RenderContext>>>,
    lib_err: Option<String>,
    events: Option<Receiver<Event>>,
    batches: Option<Receiver<Vec<PathBuf>>>,
    stop: Arc<AtomicBool>,
    dirty: Arc<AtomicBool>,
    target: Arc<Mutex<Option<crate::video::Target>>>,
    /// Ratio vidéo ×1000 transmis au rendu GL pour le mode cinéma (0 = désactivé).
    cinema_aspect: Arc<std::sync::atomic::AtomicU32>,
    cinema_blur: Arc<std::sync::atomic::AtomicU32>,
    /// Barre de commandes affichée à l'image précédente / curseur masqué (plein écran).
    bars_shown: bool,
    cursor_hidden: bool,
    /// Derushage : notes du média courant, saisie en cours, fenêtre de liste, cadence.
    notes: Vec<Note>,
    note_edit: Option<NoteEdit>,
    show_notes: bool,
    fps: f64,
    pl: Playlist,
    kind: Option<MediaKind>,
    pos: f64,
    dur: f64,
    paused: bool,
    seeking: Option<f64>,
    speed: f64,
    tracks: Vec<Track>,
    pending_seek: Option<f64>,
    eof_handled: bool,
    loaded: bool,
    show_list: bool,
    show_settings: bool,
    outside: ff_kit::outside::OutsideClose,
    close_outside: bool,
    quitting: bool,
    /// Pipette de couleur (touche I).
    pick: bool,
    probe: Arc<crate::video::Probe>,
    fullscreen: bool,
    last_move: Instant,
    toast: Option<(String, Instant)>,
    /// Message d'un enregistrement de média en cours (thread de téléchargement).
    save_msg: Arc<Mutex<Option<String>>>,
    /// Enregistrement d'un direct en cours.
    rec: Option<ff_online::Recorder>,
    rec_slot: Arc<Mutex<Option<ff_online::Recorder>>>,
    zoom: f64,
    rot: i32,
    slideshow: bool,
    slide_t: Instant,
    last_resume_save: Instant,
    meta: String,
    last_title: String,
    info: String,
    mesh: Option<MeshView>,
    doc_pages: usize,
    doc_page: usize,
    vu: [f32; 2],
    vu_peak: [f32; 2],
    theme_seen: (String, bool),
    theme_check: Instant,
}

/// Note en cours de saisie / modification.
struct NoteEdit {
    idx: Option<usize>,
    tc: String,
    dur: f64,
    text: String,
    /// La lecture était en cours avant l'ouverture : on la reprend à la validation.
    resume: bool,
    focus: bool,
}

fn fmt_t(s: f64) -> String {
    let s = s.max(0.0) as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s % 3600 / 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// Fichiers média d'un dossier (tri naturel).
fn dir_media(d: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(d)
        .map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_file() && kind_of(p).is_some()).collect())
        .unwrap_or_default();
    v.sort_by(|a, b| natural_cmp(&a.to_string_lossy(), &b.to_string_lossy()));
    v
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, listener: Option<TcpListener>, set: PlayerSettings, files: Vec<PathBuf>) -> Self {
        let ctx = cc.egui_ctx.clone();
        let shared = ff_profiles::load();
        ff_kit::theme::apply(&ctx, ff_kit::theme::Style::from_id(&shared.theme), shared.dark);
        let (mut mpv, mut render, mut lib_err, mut events) = (None, None, None, None);
        let stop = Arc::new(AtomicBool::new(false));
        let dirty = Arc::new(AtomicBool::new(true));
        let opts: Vec<(&str, &str)> = vec![
            ("vo", "libmpv"),
            ("hwdec", if set.hwdec { "auto-safe" } else { "no" }),
            ("keep-open", "yes"),
            ("idle", "yes"),
            ("image-display-duration", "inf"),
            ("osc", "no"),
            ("input-default-bindings", "no"),
            ("input-vo-keyboard", "no"),
            ("terminal", "no"),
            ("config", "no"),
            ("audio-display", "embedded-first"),
            ("sub-auto", "fuzzy"),
            // Damier derrière les images à fond transparent (ignoré par les libmpv trop anciennes).
            ("background", "tiles"),
            // Lecture des liens en ligne (YouTube…) via yt-dlp.
            ("ytdl", "yes"),
            ("ytdl-format", "bv*[height<=1080]+ba/b[height<=1080]/b"),
            ("force-window", "no"),
            ("cursor-autohide", "no"),
        ];
        let mut opts = opts;
        let ytdl_opt = ff_online::locate().filter(|p| p.is_absolute()).map(|p| format!("ytdl_hook-ytdl_path={}", p.display()));
        if let Some(o) = &ytdl_opt {
            opts.push(("script-opts", o.as_str()));
        }
        let dbg_log = std::env::var("MF_MPVLOG").ok();
        if let Some(f) = &dbg_log {
            opts.push(("log-file", f.as_str()));
            opts.push(("msg-level", "all=v"));
        }
        // Options mpv supplémentaires (diagnostic) : MEDIAFORGE_MPV_OPTS="ao=null,msg-level=all=v"
        let extra = std::env::var("MEDIAFORGE_MPV_OPTS").unwrap_or_default();
        for kv in extra.split(',').filter(|s| !s.is_empty()) {
            if let Some((k, v)) = kv.split_once('=') {
                opts.push((k, v));
            }
        }
        match Mpv::new(&opts) {
            Err(e) => lib_err = Some(e),
            Ok(m) => match cc.get_proc_address.as_ref().map(|gpa| unsafe { m.gl_render_context(&|n| gpa(n)) }) {
                None => lib_err = Some("OpenGL indisponible".into()),
                Some(Err(e)) => lib_err = Some(e),
                Some(Ok(mut rc)) => {
                    let c = ctx.clone();
                    let d2 = dirty.clone();
                    rc.set_update_callback(move || {
                        d2.store(true, Ordering::Relaxed);
                        c.request_repaint();
                    });
                    render = Some(Arc::new(Mutex::new(rc)));
                    let (tx, rx) = channel();
                    let (m2, c2, st) = (m.clone(), ctx.clone(), stop.clone());
                    std::thread::spawn(move || {
                        while !st.load(Ordering::Relaxed) {
                            match m2.wait_event(0.25) {
                                Event::Shutdown => break,
                                Event::None => {}
                                e => {
                                    if tx.send(e).is_err() {
                                        break;
                                    }
                                    c2.request_repaint();
                                }
                            }
                        }
                    });
                    events = Some(rx);
                    // Mise à l'échelle : haute qualité sur GPU, bilinéaire en rendu logiciel (llvmpipe, GDI…) ou si désactivée.
                    let soft = cc
                        .gl
                        .as_ref()
                        .map(|g| {
                            use eframe::glow::HasContext;
                            let r = unsafe { g.get_parameter_string(eframe::glow::RENDERER) }.to_lowercase();
                            ["llvmpipe", "softpipe", "swrast", "software", "generic", "basic render"].iter().any(|k| r.contains(k))
                        })
                        .unwrap_or(false);
                    if soft || !set.hq_scaling {
                        for k in ["scale", "cscale", "dscale"] {
                            let _ = m.set_property(k, "bilinear");
                        }
                    }
                    let _ = m.set_property("volume", &format!("{}", set.volume));
                    let _ = m.set_property("mute", if set.muted { "yes" } else { "no" });
                    mpv = Some(m);
                }
            },
        }
        let batches = listener.map(|l| {
            let c = ctx.clone();
            ff_core::instance::serve(l, move || {
                // Fenêtre réduite/résidente : la ramener au premier plan sans attendre une image
                c.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                c.send_viewport_cmd(egui::ViewportCommand::Focus);
                c.request_repaint()
            })
        });
        let mut pl = Playlist::default();
        pl.repeat = set.repeat;
        pl.set_shuffle(set.shuffle);
        let mut a = Self {
            l: set.lang,
            resume: Resume::load(),
            set,
            mpv,
            render,
            lib_err,
            events,
            batches,
            stop,
            dirty,
            target: Arc::new(Mutex::new(None)),
            cinema_aspect: Arc::new(std::sync::atomic::AtomicU32::new(0)),
            cinema_blur: Arc::new(std::sync::atomic::AtomicU32::new(4)),
            bars_shown: true,
            cursor_hidden: false,
            notes: vec![],
            note_edit: None,
            show_notes: false,
            fps: 25.0,
            pl,
            kind: None,
            pos: 0.0,
            dur: 0.0,
            paused: false,
            seeking: None,
            speed: 1.0,
            tracks: Vec::new(),
            pending_seek: None,
            eof_handled: false,
            loaded: false,
            show_list: false,
            info: String::new(),
            mesh: None,
            doc_pages: 1,
            doc_page: 0,
            vu: [0.0; 2],
            vu_peak: [0.0; 2],
            theme_seen: (shared.theme.clone(), shared.dark),
            theme_check: Instant::now(),
            show_settings: false,
            outside: Default::default(),
            close_outside: ff_profiles::load().close_on_outside,
            quitting: false,
            pick: false,
            probe: Arc::new(Default::default()),
            fullscreen: false,
            last_move: Instant::now(),
            toast: None,
            save_msg: Arc::new(Mutex::new(None)),
            rec: None,
            rec_slot: Arc::new(Mutex::new(None)),
            zoom: 0.0,
            rot: 0,
            slideshow: false,
            slide_t: Instant::now(),
            last_resume_save: Instant::now(),
            meta: String::new(),
            last_title: String::new(),
        };
        if !files.is_empty() {
            a.open_paths(files, false);
        }
        a
    }

    fn pick_clear(&mut self) {
        if let Ok(mut g) = self.probe.req.lock() {
            *g = None;
        }
        if let Ok(mut g) = self.probe.res.lock() {
            *g = None;
        }
    }

    /// Pipette : couleur sous le pointeur (infobulle) ; un clic copie la référence dans le presse-papiers.
    fn pick_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, r: &egui::Response) {
        if !self.pick || self.mesh.is_some() || self.pl.current().is_none() {
            return;
        }
        let pos = ctx.input(|i| i.pointer.latest_pos()).filter(|_| r.hovered());
        let Some(pos) = pos else {
            self.pick_clear();
            return;
        };
        let ppp = ctx.pixels_per_point();
        if let Ok(mut g) = self.probe.req.lock() {
            *g = Some((pos.x * ppp, pos.y * ppp));
        }
        ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
        ctx.request_repaint();
        let Some([cr, cg, cb, _]) = self.probe.res.lock().ok().and_then(|g| *g) else { return };
        let web = format!("#{cr:02X}{cg:02X}{cb:02X}");
        let rgb = format!("rgb({cr}, {cg}, {cb})");
        if r.clicked() {
            let txt = match self.set.pick_fmt {
                1 => rgb.clone(),
                2 => format!("{web} {rgb}"),
                _ => web.clone(),
            };
            ctx.copy_text(txt.clone());
            self.say(format!("{} {txt}", tr(self.l, "Couleur copiée :")));
        }
        // Infobulle : pastille + codes, repliée du côté libre près des bords.
        let (w, h) = (210.0f32, 46.0f32);
        let area = ctx.content_rect();
        let x = if pos.x + 18.0 + w > area.right() { pos.x - 18.0 - w } else { pos.x + 18.0 };
        let y = if pos.y + 18.0 + h > area.bottom() { pos.y - 18.0 - h } else { pos.y + 18.0 };
        let rc = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, h));
        let p = ui.painter();
        p.rect_filled(rc, 6.0, Color32::from_black_alpha(215));
        let sw = egui::Rect::from_min_size(rc.min + egui::vec2(6.0, 6.0), egui::vec2(34.0, 34.0));
        p.rect_filled(sw, 4.0, Color32::from_rgb(cr, cg, cb));
        p.rect_stroke(sw, 4.0, egui::Stroke::new(1.0, Color32::WHITE), egui::StrokeKind::Outside);
        p.text(rc.min + egui::vec2(48.0, 14.0), egui::Align2::LEFT_CENTER, &web, egui::FontId::monospace(14.0), Color32::WHITE);
        p.text(rc.min + egui::vec2(48.0, 32.0), egui::Align2::LEFT_CENTER, &rgb, egui::FontId::monospace(12.0), Color32::from_gray(200));
    }

    fn say(&mut self, s: impl Into<String>) {
        self.toast = Some((s.into(), Instant::now()));
    }

    // ---------- ouverture / liste ----------

    /// Ouvre des chemins : un seul fichier -> son dossier devient la liste ; sinon la sélection.
    fn open_paths(&mut self, paths: Vec<PathBuf>, append: bool) {
        let mut files: Vec<PathBuf> = Vec::new();
        for p in &paths {
            if p.to_string_lossy().starts_with("http") {
                files.push(p.clone());
            } else if p.is_dir() {
                files.extend(dir_media(p));
            } else if kind_of(p).is_some() {
                files.push(p.clone());
            }
        }
        if files.is_empty() {
            return;
        }
        if append {
            let first_new = self.pl.items.len();
            self.pl.append(files);
            if self.pl.cur.is_none() && first_new < self.pl.items.len() {
                self.play(first_new);
            }
            return;
        }
        if files.len() == 1 && paths.len() == 1 && !paths[0].is_dir() && !ff_online::is_url(&paths[0].to_string_lossy()) && self.set.load_siblings {
            let sib = siblings(&files[0]);
            if let Some(i) = sib.iter().position(|p| p == &files[0]) {
                self.pl.set(sib, i);
                self.play(i);
                return;
            }
        }
        self.pl.set(files, 0);
        self.play(0);
    }

    fn remember(&mut self) {
        if let (true, Some(p)) = (self.set.resume, self.pl.current().cloned()) {
            if self.kind != Some(MediaKind::Image) {
                self.resume.record(&p, self.pos, self.dur);
                self.resume.save();
            }
        }
    }

    fn play(&mut self, idx: usize) {
        let Some(path) = self.pl.items.get(idx).cloned() else { return };
        self.remember();
        self.pl.cur = Some(idx);
        self.kind = kind_of(&path);
        self.pos = 0.0;
        self.dur = 0.0;
        self.zoom = 0.0;
        self.rot = 0;
        self.tracks.clear();
        self.info.clear();
        self.vu = [0.0; 2];
        self.vu_peak = [0.0; 2];
        self.eof_handled = false;
        self.loaded = false;
        self.slide_t = Instant::now();
        self.note_edit = None;
        self.notes = if self.kind != Some(MediaKind::Image) && path.is_file() { derush::load(&path) } else { vec![] };
        self.pending_seek = if self.set.resume && self.kind != Some(MediaKind::Image) { self.resume.get(&path) } else { None };
        if let Some(m) = &self.mpv {
            let gif = path.extension().map(|e| e.eq_ignore_ascii_case("gif")).unwrap_or(false);
            let _ = m.set_property("loop-file", if gif || self.pl.repeat == Repeat::One { "inf" } else { "no" });
            let _ = m.set_property("video-zoom", "0");
            let _ = m.set_property("video-pan-x", "0");
            let _ = m.set_property("video-pan-y", "0");
            let _ = m.set_property("video-rotate", "0");
            let _ = m.set_property("pause", "no");
            let _ = m.set_property("af", if self.kind == Some(MediaKind::Audio) { VU_FILTER } else { "" });
        }
        self.doc_pages = 1;
        self.doc_page = 0;
        if is_doc(&path) {
            self.doc_pages = doc::page_count(&path);
        }
        self.load_current(&path);
    }

    /// Charge le fichier courant dans mpv (les documents sont d'abord rendus en PNG).
    fn load_current(&mut self, path: &Path) {
        self.mesh = None;
        if is_mesh(path) {
            match crate::mesh::load(path) {
                Ok(m) => {
                    self.info = format!("{} · {} {}", path.extension().map(|e| e.to_string_lossy().to_uppercase()).unwrap_or_default(), m.tris.len(), tr(self.l, "triangles"));
                    self.mesh = Some(MeshView { mesh: m, yaw: 0.6, pitch: 0.4, zoom: 1.0, wire: false, tex: None, size: [0, 0], dirty: true });
                    self.cmd(&["stop"]);
                }
                Err(e) => self.say(format!("{}{e}", tr(self.l, "Erreur de lecture : "))),
            }
            return;
        }
        let target = if is_doc(path) {
            match doc::render(path, self.doc_page) {
                Ok(png) => png,
                Err(e) => {
                    self.say(format!("{}{e}", tr(self.l, "Erreur de lecture : ")));
                    return;
                }
            }
        } else if ff_chip::is_chip(path) {
            // Musiques Atari ST / C64 : rendu WAV (mis en cache dans le dossier temporaire), puis lecture normale.
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            path.hash(&mut h);
            std::fs::metadata(path).and_then(|m| m.modified()).ok().hash(&mut h);
            let wav = std::env::temp_dir().join(format!("mediaforge_player_chip_{:016x}.wav", h.finish()));
            if !wav.exists() {
                if let Err(e) = ff_chip::render_to_wav(path, &wav, 240.0) {
                    self.say(format!("{}{e}", tr(self.l, "Erreur de lecture : ")));
                    return;
                }
            }
            wav
        } else {
            path.to_path_buf()
        };
        if let Some(m) = &self.mpv {
            if let Err(e) = m.command(&["loadfile", &target.to_string_lossy()]) {
                self.say(format!("{}{e}", tr(self.l, "Erreur de lecture : ")));
            }
        }
    }

    fn doc_goto(&mut self, delta: i32) {
        let n = self.doc_pages as i32;
        let np = (self.doc_page as i32 + delta).clamp(0, n - 1) as usize;
        if np != self.doc_page {
            self.doc_page = np;
            self.zoom = 0.0;
            if let Some(p) = self.pl.current().cloned() {
                self.load_current(&p);
            }
        }
    }

    fn step(&mut self, next: bool, auto: bool) {
        let i = if next { self.pl.next_index(auto) } else { self.pl.prev_index() };
        if let Some(i) = i {
            self.play(i);
        }
    }

    // ---------- commandes mpv ----------

    fn cmd(&self, a: &[&str]) {
        if let Some(m) = &self.mpv {
            let _ = m.command(a);
        }
    }
    fn prop(&self, k: &str, v: &str) {
        if let Some(m) = &self.mpv {
            let _ = m.set_property(k, v);
        }
    }
    fn seek_rel(&self, s: f64) {
        self.cmd(&["seek", &s.to_string(), "relative"]);
    }
    fn toggle_pause(&mut self) {
        self.prop("pause", if self.paused { "no" } else { "yes" });
        self.paused = !self.paused;
    }
    fn set_volume(&mut self, v: f64) {
        self.set.volume = v.clamp(0.0, 130.0);
        self.prop("volume", &self.set.volume.to_string());
    }
    fn set_speed(&mut self, s: f64) {
        self.speed = s.clamp(0.25, 4.0);
        self.prop("speed", &self.speed.to_string());
    }
    fn is_image(&self) -> bool {
        self.kind == Some(MediaKind::Image)
    }
    fn apply_zoom(&mut self, z: f64) {
        self.zoom = z.clamp(-3.0, 6.0);
        self.prop("video-zoom", &self.zoom.to_string());
    }
    /// Taille réelle : zoom (log2) qui annule l'échelle d'ajustement à la fenêtre.
    fn actual_size(&mut self, ctx: &egui::Context) {
        let (vw, vh) = match &self.mpv {
            Some(m) => (m.get_f64("width").unwrap_or(0.0), m.get_f64("height").unwrap_or(0.0)),
            None => return,
        };
        let r = ctx.content_rect().size() * ctx.pixels_per_point();
        if vw > 0.0 && vh > 0.0 {
            let fit = (r.x as f64 / vw).min(r.y as f64 / vh);
            self.apply_zoom(-fit.log2());
        }
    }
    fn fit(&mut self) {
        self.apply_zoom(0.0);
        self.prop("video-pan-x", "0");
        self.prop("video-pan-y", "0");
    }
    fn rotate(&mut self) {
        self.rot = (self.rot + 90) % 360;
        self.prop("video-rotate", &self.rot.to_string());
    }
    fn set_fullscreen(&mut self, ctx: &egui::Context, on: bool) {
        self.fullscreen = on;
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(on));
        self.last_move = Instant::now();
        if !on && self.cursor_hidden {
            self.cursor_hidden = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::CursorVisible(true));
        }
    }

    fn screenshot_to(&mut self, path: &Path) -> Result<(), String> {
        let m = self.mpv.as_ref().ok_or("mpv")?;
        m.command(&["screenshot-to-file", &path.to_string_lossy(), "video"])
    }

    fn capture(&mut self) {
        let dir = dirs::picture_dir().unwrap_or_else(std::env::temp_dir);
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let p = dir.join(format!("MediaForge-{secs}.png"));
        match self.screenshot_to(&p) {
            Ok(()) => self.say(format!("{}{}", tr(self.l, "Capture enregistrée : "), p.display())),
            Err(e) => self.say(format!("{}{e}", tr(self.l, "Capture impossible : "))),
        }
    }

    /// Enregistre le média affiché : copie du fichier local, ou téléchargement (yt-dlp) pour une URL.
    fn save_media(&mut self) {
        let Some(p) = self.pl.current().cloned() else { return };
        let s = p.to_string_lossy().to_string();
        let l = self.l;
        // Arrêt d'un enregistrement de direct en cours : finalisation du MP4 en arrière-plan.
        if let Some(mut rec) = self.rec.take() {
            let slot = self.save_msg.clone();
            self.say(tr(l, "Finalisation du fichier…"));
            std::thread::spawn(move || {
                let m = match rec.stop() {
                    Ok(f) => format!("{}{}", tr(l, "Média enregistré : "), f.display()),
                    Err(e) => format!("{}{e}", tr(l, "Enregistrement impossible : ")),
                };
                *slot.lock().unwrap() = Some(m);
            });
            return;
        }
        if ff_online::is_url(&s) {
            let dir = dirs::download_dir().unwrap_or_else(std::env::temp_dir);
            let ffd = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("ffmpeg"))).filter(|d| d.exists());
            let slot = self.save_msg.clone();
            self.say(tr(l, "Téléchargement en cours…"));
            let rslot = self.rec_slot.clone();
            std::thread::spawn(move || {
                // Direct : enregistrement continu (à arrêter en recliquant) ; sinon téléchargement normal.
                if ff_online::is_live(&s) {
                    let m = match ff_online::Recorder::start(&s, &dir, ffd.as_deref()) {
                        Ok(r) => {
                            *rslot.lock().unwrap() = Some(r);
                            tr(l, "Enregistrement du direct… recliquez pour arrêter et finaliser.").to_string()
                        }
                        Err(e) => format!("{}{e}", tr(l, "Enregistrement impossible : ")),
                    };
                    *slot.lock().unwrap() = Some(m);
                    return;
                }
                let r = ff_online::download(&s, &dir, ffd.as_deref(), ff_online::Quality::Video1080, &mut |_| {});
                let m = match r {
                    Ok(f) => format!("{}{}", tr(l, "Média enregistré : "), f.display()),
                    Err(e) => format!("{}{e}", tr(l, "Enregistrement impossible : ")),
                };
                *slot.lock().unwrap() = Some(m);
            });
        } else if p.is_file() {
            let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            if let Some(dst) = rfd::FileDialog::new().set_file_name(name).save_file() {
                match std::fs::copy(&p, &dst) {
                    Ok(_) => self.say(format!("{}{}", tr(l, "Média enregistré : "), dst.display())),
                    Err(e) => self.say(format!("{}{e}", tr(l, "Enregistrement impossible : "))),
                }
            }
        }
    }

    fn copy_image(&mut self) {
        let tmp = std::env::temp_dir().join("mediaforge-clip.png");
        let r = self.screenshot_to(&tmp).and_then(|_| {
            let img = image::open(&tmp).map_err(|e| e.to_string())?.to_rgba8();
            let (w, h) = (img.width() as usize, img.height() as usize);
            arboard::Clipboard::new()
                .and_then(|mut c| c.set_image(arboard::ImageData { width: w, height: h, bytes: img.into_raw().into() }))
                .map_err(|e| e.to_string())
        });
        let _ = std::fs::remove_file(&tmp);
        match r {
            Ok(()) => self.say(tr(self.l, "Image copiée dans le presse-papiers.")),
            Err(e) => self.say(format!("{}{e}", tr(self.l, "Copie impossible : "))),
        }
    }

    fn open_dialog(&mut self, append: bool) {
        let exts: Vec<&str> = ff_core::media::VIDEO_EXT.iter().chain(ff_core::media::AUDIO_EXT).chain(ff_core::media::IMAGE_EXT).chain(ff_core::media::DOC_EXT).chain(ff_core::media::MESH_EXT).copied().collect();
        if let Some(f) = rfd::FileDialog::new().add_filter(tr(self.l, "Médias"), &exts).pick_files() {
            self.open_paths(f, append);
        }
    }

    fn convert_current(&self) {
        if let Some(p) = self.pl.current() {
            let exe = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join(if cfg!(windows) { "mediaforge.exe" } else { "mediaforge" })));
            let bin = exe.filter(|e| e.exists()).unwrap_or_else(|| PathBuf::from("mediaforge"));
            let _ = std::process::Command::new(bin).arg(p).spawn();
        }
    }

    fn reveal(&self) {
        if let Some(p) = self.pl.current() {
            #[cfg(windows)]
            let _ = std::process::Command::new("explorer").arg("/select,").arg(p).spawn();
            #[cfg(not(windows))]
            let _ = std::process::Command::new("xdg-open").arg(p.parent().unwrap_or(p)).spawn();
        }
    }

    // ---------- état mpv ----------

    fn refresh_tracks(&mut self) {
        let Some(m) = &self.mpv else { return };
        self.tracks.clear();
        let n = m.get_i64("track-list/count").unwrap_or(0);
        for i in 0..n {
            let ty = m.get_string(&format!("track-list/{i}/type")).unwrap_or_default();
            if ty != "audio" && ty != "sub" {
                continue;
            }
            let id = m.get_i64(&format!("track-list/{i}/id")).unwrap_or(0);
            let lang = m.get_string(&format!("track-list/{i}/lang")).unwrap_or_default();
            let title = m.get_string(&format!("track-list/{i}/title")).unwrap_or_default();
            let sel = m.get_bool(&format!("track-list/{i}/selected")).unwrap_or(false);
            let mut label = format!("#{id}");
            for s in [&lang, &title] {
                if !s.is_empty() {
                    label.push_str(" · ");
                    label.push_str(s);
                }
            }
            self.tracks.push(Track { id, audio: ty == "audio", label, selected: sel });
        }
        // Caractéristiques du fichier (barre du bas) : définition, codecs, image/s, taille.
        let mut parts: Vec<String> = Vec::new();
        let mut audio_done = false;
        for i in 0..n {
            let ty = m.get_string(&format!("track-list/{i}/type")).unwrap_or_default();
            let codec = m.get_string(&format!("track-list/{i}/codec")).unwrap_or_default().to_uppercase();
            let t = |k: &str| m.get_i64(&format!("track-list/{i}/{k}"));
            if ty == "video" && !m.get_bool(&format!("track-list/{i}/albumart")).unwrap_or(false) {
                let mut v = codec.clone();
                if let (Some(w), Some(h)) = (t("demux-w"), t("demux-h")) {
                    v.push_str(&format!(" · {w}×{h}"));
                }
                if let Some(f) = m.get_f64(&format!("track-list/{i}/demux-fps")).filter(|f| *f > 0.0 && self.kind == Some(MediaKind::Video)) {
                    v.push_str(&format!(" · {} fps", (f * 100.0).round() / 100.0));
                }
                parts.push(v);
            } else if ty == "audio" && !audio_done {
                audio_done = true;
                let mut a = codec.clone();
                if let Some(sr) = t("demux-samplerate") {
                    a.push_str(&format!(" · {} kHz", (sr as f64 / 100.0).round() / 10.0));
                }
                if let Some(c) = t("demux-channel-count") {
                    a.push_str(match c { 1 => " · mono", 2 => " · stéréo", _ => "" });
                }
                if let Some(b) = t("demux-bitrate").filter(|b| *b > 0) {
                    a.push_str(&format!(" · {} kb/s", b / 1000));
                }
                parts.push(a);
            }
        }
        if let Some(sz) = m.get_f64("file-size") {
            parts.push(if sz >= 1048576.0 { format!("{:.1} Mo", sz / 1048576.0) } else { format!("{:.0} Ko", sz / 1024.0) });
        }
        let prefix = match self.pl.current() {
            Some(p) if is_doc(p) => format!("{} · {}/{} · ", p.extension().map(|e| e.to_string_lossy().to_uppercase()).unwrap_or_default(), self.doc_page + 1, self.doc_pages),
            _ => String::new(),
        };
        self.info = format!("{prefix}{}", parts.join("   |   "));
        let g = |k: &str| m.get_string(&format!("metadata/by-key/{k}")).unwrap_or_default();
        let (t, a, al) = (g("title"), g("artist"), g("album"));
        self.meta = [a, t, al].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n");
    }

    /// Suit le thème partagé avec MediaForge (relu toutes les 1,5 s).
    fn sync_theme(&mut self, ctx: &egui::Context) {
        if self.theme_check.elapsed() < Duration::from_millis(1500) {
            return;
        }
        self.theme_check = Instant::now();
        let sh = ff_profiles::load();
        if (sh.theme.clone(), sh.dark) != self.theme_seen {
            ff_kit::theme::apply(ctx, ff_kit::theme::Style::from_id(&sh.theme), sh.dark);
            self.theme_seen = (sh.theme, sh.dark);
        }
        self.close_outside = sh.close_on_outside;
    }

    /// Niveaux du vumètre (audio uniquement) avec retombée douce et maintien de crête.
    fn poll_vu(&mut self, ctx: &egui::Context) {
        if self.kind != Some(MediaKind::Audio) || self.paused || !self.loaded {
            return;
        }
        if let Some(v) = self.mpv.as_ref().and_then(|m| m.get_string("af-metadata/vu")).and_then(|j| parse_vu(&j)) {
            for i in 0..2 {
                self.vu[i] = if v[i] > self.vu[i] { v[i] } else { self.vu[i] * 0.8 + v[i] * 0.2 };
                self.vu_peak[i] = self.vu[i].max(self.vu_peak[i] - 0.015);
            }
        }
        ctx.request_repaint_after(Duration::from_millis(50));
    }

    fn pump(&mut self, ctx: &egui::Context) {
        self.sync_theme(ctx);
        self.poll_vu(ctx);
        let evs: Vec<Event> = self.events.as_ref().map(|r| r.try_iter().collect()).unwrap_or_default();
        for e in evs {
            match e {
                Event::StartFile => self.eof_handled = false,
                Event::FileLoaded => {
                    self.loaded = true;
                    self.refresh_tracks();
                    if let Some(p) = self.pending_seek.take() {
                        self.cmd(&["seek", &p.to_string(), "absolute"]);
                        let msg = format!("{}{}", tr(self.l, "Reprise à "), fmt_t(p));
                        self.say(msg);
                    }
                }
                Event::EndFile(EndReason::Error) => {
                    let n = self.pl.current().and_then(|p| p.file_name()).map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                    self.say(format!("{}{n}", tr(self.l, "Erreur de lecture : ")));
                    if self.pl.items.len() > 1 {
                        self.step(true, true);
                    }
                }
                _ => {}
            }
        }
        if let Some(rx) = &self.batches {
            let got: Vec<Vec<PathBuf>> = rx.try_iter().collect();
            for b in got {
                self.open_paths(b, false);
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
        }
        if self.mpv.is_none() {
            return;
        }
        if !self.is_image() {
            if let Some(m) = &self.mpv {
                self.pos = m.get_f64("time-pos").unwrap_or(self.pos);
                if let Some(f) = m.get_f64("container-fps").or_else(|| m.get_f64("estimated-vf-fps")).filter(|f| *f > 1.0) {
                    self.fps = f;
                }
                self.dur = m.get_f64("duration").unwrap_or(self.dur);
                self.paused = m.get_bool("pause").unwrap_or(false);
                // Fin naturelle (keep-open) : passage au suivant.
                if self.loaded && !self.eof_handled && m.get_bool("eof-reached").unwrap_or(false) && self.kind.is_some() {
                    self.eof_handled = true;
                    let nxt = self.pl.next_index(true);
                    if let Some(i) = nxt {
                        if self.pl.repeat == Repeat::One {
                            let _ = m.command(&["seek", "0", "absolute"]);
                            let _ = m.set_property("pause", "no");
                            self.eof_handled = false;
                        } else {
                            self.play(i);
                        }
                    }
                }
            }
            if self.last_resume_save.elapsed() > Duration::from_secs(15) {
                self.last_resume_save = Instant::now();
                self.remember();
            }
        } else if self.slideshow && self.slide_t.elapsed() >= Duration::from_secs(self.set.slideshow_secs.max(1) as u64) {
            self.slide_t = Instant::now();
            match self.pl.next_index(true) {
                Some(i) => self.play(i),
                None => self.slideshow = false,
            }
        }
        // Titre de fenêtre
        let name = self.pl.current().and_then(|p| p.file_name()).map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let title = if name.is_empty() { "MediaForge Player".to_string() } else { format!("{name} - MediaForge Player") };
        if title != self.last_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }
        if (!self.paused && !self.is_image()) || self.slideshow {
            ctx.request_repaint_after(Duration::from_millis(200));
        }
    }

    // ---------- clavier ----------

    fn keys(&mut self, ctx: &egui::Context) {
        // Saisie de texte en cours (note, réglages) : pas de raccourcis.
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let (ctrl, shift) = ctx.input(|i| (i.modifiers.command, i.modifiers.shift));
        let pressed = |k: Key| ctx.input(|i| i.key_pressed(k));
        let big = if ctrl { 30.0 } else { 5.0 };
        if pressed(Key::W) {
            if let Some(mv) = &mut self.mesh {
                mv.wire = !mv.wire;
                mv.dirty = true;
            }
        }
        if pressed(Key::Space) {
            // Image : Espace passe à la suivante ; vidéo/audio : lecture/pause
            if self.is_image() {
                self.step(true, false);
            } else {
                self.toggle_pause();
            }
        }
        if pressed(Key::ArrowRight) {
            if self.doc_pages > 1 { self.doc_goto(1) } else if self.is_image() { self.step(true, false) } else { self.seek_rel(big) }
        }
        if pressed(Key::ArrowLeft) {
            if self.doc_pages > 1 { self.doc_goto(-1) } else if self.is_image() { self.step(false, false) } else { self.seek_rel(-big) }
        }
        if pressed(Key::ArrowUp) {
            self.set_volume(self.set.volume + 5.0);
        }
        if pressed(Key::ArrowDown) {
            self.set_volume(self.set.volume - 5.0);
        }
        if pressed(Key::M) {
            self.set.muted = !self.set.muted;
            self.prop("mute", if self.set.muted { "yes" } else { "no" });
        }
        if pressed(Key::F) || pressed(Key::F11) || (pressed(Key::Enter) && ctx.input(|i| i.modifiers.alt)) {
            self.set_fullscreen(ctx, !self.fullscreen);
        }
        if !ctrl && self.kind != Some(MediaKind::Image) && self.pl.current().is_some() {
            if pressed(Key::T) {
                self.set.tc_on = !self.set.tc_on;
            }
            if pressed(Key::A) {
                self.begin_note(None);
            }
            if pressed(Key::J) {
                self.show_notes = !self.show_notes;
            }
            // Image par image (derushage) : « . » avance, « , » recule.
            if pressed(Key::Period) {
                self.cmd(&["frame-step"]);
            }
            if pressed(Key::Comma) {
                self.cmd(&["frame-back-step"]);
            }
        }
        if pressed(Key::I) && !ctrl && self.mesh.is_none() && self.pl.current().is_some() {
            self.pick = !self.pick;
            if !self.pick {
                self.pick_clear();
            }
        }
        if pressed(Key::Escape) && self.pick {
            self.pick = false;
            self.pick_clear();
        }
        if pressed(Key::C) && !ctx.input(|i| i.modifiers.command) {
            self.set.cinema = !self.set.cinema;
            let m = if self.set.cinema { "Mode cinéma activé" } else { "Mode cinéma désactivé" };
            self.say(tr(self.l, m).to_string());
        }
        if pressed(Key::Escape) && self.fullscreen {
            self.set_fullscreen(ctx, false);
        }
        if pressed(Key::N) || pressed(Key::PageDown) {
            self.step(true, false);
        }
        if pressed(Key::P) || pressed(Key::PageUp) {
            self.step(false, false);
        }
        if pressed(Key::OpenBracket) {
            let s = self.speed - 0.25;
            self.set_speed(s);
        }
        if pressed(Key::CloseBracket) {
            let s = self.speed + 0.25;
            self.set_speed(s);
        }
        if pressed(Key::S) {
            self.capture();
        }
        if pressed(Key::C) && ctrl {
            self.copy_image();
        }
        if pressed(Key::R) {
            self.rotate();
        }
        if pressed(Key::Plus) || pressed(Key::Equals) {
            self.apply_zoom(self.zoom + 0.25);
        }
        if pressed(Key::Minus) {
            self.apply_zoom(self.zoom - 0.25);
        }
        if pressed(Key::Num0) {
            self.fit();
        }
        if pressed(Key::Num1) {
            self.actual_size(ctx);
        }
        if pressed(Key::L) {
            self.show_list = !self.show_list;
        }
        if pressed(Key::O) && ctrl {
            self.open_dialog(false);
        } else if pressed(Key::O) {
            self.open_dialog(shift);
        }
        if pressed(Key::Home) && !self.is_image() {
            self.cmd(&["seek", "0", "absolute"]);
        }
    }

    // ---------- derushage : notes horodatées ----------

    fn save_notes(&mut self) {
        let Some(p) = self.pl.current().cloned().filter(|p| p.is_file()) else { return };
        if let Err(e) = derush::save(&p, &self.notes) {
            self.say(e);
        }
    }

    /// Ouvre la saisie d'une note au temps courant (ou modifie la note `idx`) ; met la lecture en pause.
    fn begin_note(&mut self, idx: Option<usize>) {
        if self.pl.current().map(|p| !p.is_file()).unwrap_or(true) {
            self.say(tr(self.l, "Notes disponibles pour les fichiers locaux uniquement"));
            return;
        }
        let resume = idx.is_none() && !self.paused;
        if resume {
            self.toggle_pause();
        }
        let (t, dur, text) = match idx.and_then(|i| self.notes.get(i)) {
            Some(n) => (n.t, n.dur, n.text.clone()),
            None => (self.pos, self.set.notes_dur as f64, String::new()),
        };
        self.note_edit = Some(NoteEdit { idx, tc: derush::millis(t), dur, text, resume, focus: true });
    }

    fn note_dialog(&mut self, ctx: &egui::Context) {
        let l = self.l;
        let fps = self.fps;
        let Some(ed) = &mut self.note_edit else { return };
        let (mut ok, mut cancel) = (false, false);
        egui::Window::new(tr(l, "Note de derushage")).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
            egui::Grid::new("note_grid").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                ui.label(tr(l, "Timecode"));
                ui.add(egui::TextEdit::singleline(&mut ed.tc).desired_width(140.0).font(egui::TextStyle::Monospace));
                ui.end_row();
                ui.label(tr(l, "Durée d'affichage (s)"));
                ui.add(egui::DragValue::new(&mut ed.dur).range(0.5..=600.0).speed(0.1));
                ui.end_row();
                ui.label(tr(l, "Note"));
                let r = ui.add(egui::TextEdit::singleline(&mut ed.text).desired_width(360.0).hint_text(tr(l, "100 caractères maximum")));
                if ed.focus {
                    r.request_focus();
                    ed.focus = false;
                }
                if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    ok = true;
                }
                ui.end_row();
            });
            ed.text = derush::clean(&ed.text);
            ui.label(RichText::new(format!("{}/{}", ed.text.chars().count(), derush::MAX_NOTE)).small().weak());
            ui.horizontal(|ui| {
                if ui.button(tr(l, "Enregistrer")).clicked() {
                    ok = true;
                }
                if ui.button(tr(l, "Annuler")).clicked() {
                    cancel = true;
                }
            });
        });
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            cancel = true;
        }
        let tc_ok = derush::parse_tc(&ed.tc, fps);
        if ok && (tc_ok.is_none() || ed.text.trim().is_empty()) {
            ok = false;
            self.toast = Some((tr(l, "Timecode ou texte invalide").to_string(), Instant::now()));
        }
        if ok || cancel {
            let ed = self.note_edit.take().unwrap();
            if ok {
                let n = Note { t: tc_ok.unwrap_or(0.0), dur: ed.dur, text: ed.text.trim().to_string() };
                match ed.idx.filter(|i| *i < self.notes.len()) {
                    Some(i) => self.notes[i] = n,
                    None => self.notes.push(n),
                }
                self.notes.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap_or(std::cmp::Ordering::Equal));
                self.save_notes();
            }
            if ed.resume && self.paused {
                self.toggle_pause();
            }
        }
    }

    fn notes_window(&mut self, ctx: &egui::Context) {
        if !self.show_notes {
            return;
        }
        let l = self.l;
        let mut open = true;
        let (mut seek, mut edit, mut del): (Option<f64>, Option<usize>, Option<usize>) = (None, None, None);
        let mut changed = false;
        egui::Window::new(tr(l, "Notes de derushage")).open(&mut open).default_size([460.0, 320.0]).show(ctx, |ui| {
            ff_kit::help::button(ui, "player", "derushage-notes", "?");
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.set.notes_show, tr(l, "Afficher sur l'image"));
                ui.label(tr(l, "Durée par défaut (s)"));
                ui.add(egui::DragValue::new(&mut self.set.notes_dur).range(0.5..=600.0).speed(0.1));
                if ui.button(tr(l, "+ Note ici (A)")).clicked() {
                    self.note_edit = None;
                    self.begin_note(None);
                }
            });
            ui.separator();
            if self.notes.is_empty() {
                ui.label(RichText::new(tr(l, "Aucune note. Touche A : annoter l'instant courant.")).weak());
            }
            egui::ScrollArea::vertical().show(ui, |ui| {
                for (i, n) in self.notes.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        if ui.link(RichText::new(derush::smpte(n.t, self.fps, false)).monospace()).on_hover_text(tr(l, "Aller à cet instant")).clicked() {
                            seek = Some(n.t);
                        }
                        if ui.add(egui::DragValue::new(&mut n.dur).range(0.5..=600.0).speed(0.1).suffix(" s")).changed() {
                            changed = true;
                        }
                        ui.label(&n.text);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("✕").clicked() {
                                del = Some(i);
                            }
                            if ui.small_button("✎").clicked() {
                                edit = Some(i);
                            }
                        });
                    });
                }
            });
            if let Some(p) = self.pl.current().filter(|p| p.is_file()) {
                ui.separator();
                ui.label(RichText::new(derush::sidecar(p).display().to_string()).small().weak());
            }
        });
        if let Some(t) = seek {
            self.cmd(&["seek", &t.to_string(), "absolute+exact"]);
        }
        if let Some(i) = edit {
            self.begin_note(Some(i));
        }
        if let Some(i) = del {
            self.notes.remove(i);
            changed = true;
        }
        if changed {
            self.save_notes();
        }
        if !open {
            self.show_notes = false;
        }
    }

    /// Surimpression : timecodes (coin choisi) et notes actives (bas de l'image).
    fn derush_overlay(&self, ui: &egui::Ui) {
        if self.kind == Some(MediaKind::Image) || self.kind.is_none() || self.pl.current().is_none() {
            return;
        }
        let rc = ui.max_rect();
        let p = ui.painter();
        let s = &self.set;
        if s.tc_on {
            let (pos, fps) = (self.seeking.unwrap_or(self.pos), self.fps);
            let mut lines: Vec<String> = vec![];
            if s.tc_smpte {
                lines.push(derush::smpte(pos, fps, false));
            }
            if s.tc_drop {
                lines.push(derush::smpte(pos, fps, true));
            }
            if s.tc_frames {
                lines.push(format!("F {}", derush::frame_no(pos, fps)));
            }
            if s.tc_ms {
                lines.push(derush::millis(pos));
            }
            if s.tc_remain && self.dur > 0.0 {
                lines.push(format!("-{}", derush::millis((self.dur - pos).max(0.0))));
            }
            if !lines.is_empty() {
                let font = egui::FontId::monospace(s.tc_size.clamp(10.0, 80.0));
                let galleys: Vec<_> = lines.iter().map(|t| p.layout_no_wrap(t.clone(), font.clone(), Color32::WHITE)).collect();
                let w = galleys.iter().map(|g| g.size().x).fold(0.0, f32::max) + 16.0;
                let h: f32 = galleys.iter().map(|g| g.size().y).sum::<f32>() + 10.0;
                let m = 14.0;
                let x = if s.tc_corner % 2 == 0 { rc.left() + m } else { rc.right() - m - w };
                let y = if s.tc_corner < 2 { rc.top() + m } else { rc.bottom() - m - h };
                let r = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, h));
                p.rect_filled(r, 6.0, Color32::from_black_alpha(170));
                let mut yy = y + 5.0;
                for g in galleys {
                    let hh = g.size().y;
                    p.galley(egui::pos2(x + 8.0, yy), g, Color32::WHITE);
                    yy += hh;
                }
            }
        }
        if s.notes_show {
            let act = derush::active(&self.notes, self.pos);
            let mut y = rc.bottom() - 24.0;
            for n in act.iter().rev() {
                let g = p.layout(n.text.clone(), egui::FontId::proportional(22.0), Color32::WHITE, (rc.width() * 0.8).max(120.0));
                let (w, h) = (g.size().x + 20.0, g.size().y + 12.0);
                let r = egui::Rect::from_min_size(egui::pos2(rc.center().x - w / 2.0, y - h), egui::vec2(w, h));
                p.rect_filled(r, 8.0, Color32::from_rgba_unmultiplied(0, 0, 0, 190));
                p.rect_stroke(r, 8.0, egui::Stroke::new(1.5, ff_kit::theme::current().accent), egui::StrokeKind::Inside);
                p.galley(r.min + egui::vec2(10.0, 6.0), g, Color32::WHITE);
                y -= h + 6.0;
            }
        }
    }

    // ---------- éléments d'interface ----------

    fn controls(&mut self, ui: &mut egui::Ui) {
        let l = self.l;
        let ctx = ui.ctx().clone();
        let img = self.is_image();
        if !img {
            ui.horizontal(|ui| {
                ui.label(RichText::new(fmt_t(self.seeking.unwrap_or(self.pos))).monospace());
                let mut v = self.seeking.unwrap_or(self.pos);
                ui.spacing_mut().slider_width = (ui.available_width() - 80.0).max(60.0);
                let r = ui.add(egui::Slider::new(&mut v, 0.0..=self.dur.max(1.0)).show_value(false));
                if r.dragged() {
                    self.seeking = Some(v);
                } else if r.drag_stopped() || (r.changed() && self.seeking.is_none()) {
                    self.cmd(&["seek", &v.to_string(), "absolute"]);
                    self.seeking = None;
                }
                ui.label(RichText::new(fmt_t(self.dur)).monospace());
            });
        }
        ui.horizontal(|ui| {
            let b = |ui: &mut egui::Ui, ic: Icon, tip: &str| icons::button(ui, ic, 18.0, false, tip).clicked();
            if b(ui, Icon::Prev, tr(l, "Précédent")) {
                self.step(false, false);
            }
            if !img {
                let (g, tip) = if self.paused { (Icon::Play, "Lecture") } else { (Icon::Pause, "Pause") };
                if b(ui, g, tr(l, tip)) {
                    self.toggle_pause();
                }
            }
            if b(ui, Icon::Next, tr(l, "Suivant")) {
                self.step(true, false);
            }
            ui.separator();
            if img {
                if self.doc_pages > 1 {
                    if b(ui, Icon::Prev, tr(l, "Page précédente")) {
                        self.doc_goto(-1);
                    }
                    ui.label(RichText::new(format!("{}/{}", self.doc_page + 1, self.doc_pages)).monospace());
                    if b(ui, Icon::Next, tr(l, "Page suivante")) {
                        self.doc_goto(1);
                    }
                    ui.separator();
                }
                if b(ui, Icon::Fit, tr(l, "Ajuster")) {
                    self.fit();
                }
                if ui.add(egui::Button::new(RichText::new("1:1").strong()).min_size(egui::vec2(34.0, 26.0))).on_hover_text(tr(l, "Taille réelle (1:1)")).clicked() {
                    self.actual_size(&ctx);
                }
                if b(ui, Icon::ZoomIn, tr(l, "Zoom +")) {
                    self.apply_zoom(self.zoom + 0.5);
                }
                if b(ui, Icon::ZoomOut, tr(l, "Zoom −")) {
                    self.apply_zoom(self.zoom - 0.5);
                }
                if b(ui, Icon::Rotate, tr(l, "Pivoter")) {
                    self.rotate();
                }
                if icons::button(ui, Icon::Slideshow, 18.0, self.slideshow, tr(l, "Diaporama")).clicked() {
                    self.slideshow = !self.slideshow;
                    self.slide_t = Instant::now();
                }
                if b(ui, Icon::Copy, tr(l, "Copier l'image")) {
                    self.copy_image();
                }
            } else {
                let mute = if self.set.muted || self.set.volume <= 0.0 { Icon::Mute } else { Icon::Volume };
                if b(ui, mute, tr(l, "Muet")) {
                    self.set.muted = !self.set.muted;
                    self.prop("mute", if self.set.muted { "yes" } else { "no" });
                }
                let mut v = self.set.volume;
                ui.spacing_mut().slider_width = 90.0;
                if ui.add(egui::Slider::new(&mut v, 0.0..=130.0).show_value(false)).on_hover_text(tr(l, "Volume")).changed() {
                    self.set_volume(v);
                }
                ui.separator();
                egui::ComboBox::from_id_salt("speed").width(64.0).selected_text(format!("{}×", self.speed)).show_ui(ui, |ui| {
                    for &s in SPEEDS {
                        if ui.selectable_label((self.speed - s).abs() < 1e-6, format!("{s}×")).clicked() {
                            self.set_speed(s);
                        }
                    }
                });
                if self.tracks.iter().any(|t| t.audio) || self.tracks.iter().any(|t| !t.audio) || !img {
                    self.track_menu(ui);
                }
                if b(ui, Icon::Camera, tr(l, "Capture d'image")) {
                    self.capture();
                }
                if b(ui, Icon::Folder, tr(l, "Enregistrer le média")) {
                    self.save_media();
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if b(ui, if self.fullscreen { Icon::Windowed } else { Icon::Fullscreen }, tr(l, if self.fullscreen { "Quitter le plein écran" } else { "Plein écran" })) {
                    self.set_fullscreen(&ctx, !self.fullscreen);
                }
                if !img {
                    if icons::button(ui, Icon::Doc, 18.0, self.show_notes, tr(l, "Notes de derushage (J)")).clicked() {
                        self.show_notes = !self.show_notes;
                    }
                    if icons::button(ui, Icon::Pencil, 18.0, false, tr(l, "Annoter l'instant courant (A)")).clicked() {
                        self.begin_note(None);
                    }
                    if icons::button(ui, Icon::Clock, 18.0, self.set.tc_on, tr(l, "Timecode en surimpression (T)")).clicked() {
                        self.set.tc_on = !self.set.tc_on;
                    }
                }
                if icons::button(ui, Icon::Sliders, 18.0, self.show_settings, tr(l, "Réglages")).clicked() {
                    self.show_settings = !self.show_settings;
                }
                ff_kit::help::button(ui, "player", "lecture", tr(l, "Aide (F1)"));
                if icons::button(ui, Icon::List, 18.0, self.show_list, tr(l, "Liste de lecture")).clicked() {
                    self.show_list = !self.show_list;
                }
                let rep = match self.pl.repeat {
                    Repeat::Off => "Répéter : non",
                    Repeat::All => "Répéter : tout",
                    Repeat::One => "Répéter : un",
                };
                let rb = icons::button(ui, Icon::Repeat, 18.0, self.pl.repeat != Repeat::Off, &format!("{}{}", tr(l, rep), if self.pl.repeat == Repeat::One { " (1)" } else { "" }));
                if self.pl.repeat == Repeat::One {
                    // Pastille « 1 » : le fichier en cours est rejoué en boucle
                    ui.painter().text(rb.rect.right_top() + egui::vec2(-3.0, 4.0), egui::Align2::RIGHT_CENTER, "1", egui::FontId::proportional(11.0), ff_kit::theme::current().accent);
                }
                if rb.clicked() {
                    self.pl.repeat = self.pl.repeat.cycle();
                    self.set.repeat = self.pl.repeat;
                    // Boucle native mpv sur le fichier en cours (sans coupure) ; GIF toujours en boucle
                    if let Some(m) = &self.mpv {
                        let gif = self.pl.current().map(|p| p.extension().map(|e| e.eq_ignore_ascii_case("gif")).unwrap_or(false)).unwrap_or(false);
                        let _ = m.set_property("loop-file", if gif || self.pl.repeat == Repeat::One { "inf" } else { "no" });
                    }
                }
                if icons::button(ui, Icon::Shuffle, 18.0, self.pl.shuffle, tr(l, "Aléatoire")).clicked() {
                    let on = !self.pl.shuffle;
                    self.pl.set_shuffle(on);
                    self.set.shuffle = on;
                }
            });
        });
        if !self.info.is_empty() {
            ui.label(RichText::new(&self.info).small().weak());
        }
    }

    fn track_menu(&mut self, ui: &mut egui::Ui) {
        let l = self.l;
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(30.0, 26.0), egui::Sense::click());
        icons::paint(ui.painter(), egui::Rect::from_center_size(rect.center(), egui::vec2(18.0, 18.0)), Icon::Subs, if resp.hovered() { Color32::WHITE } else { Color32::from_gray(215) });
        egui::Popup::menu(&resp).show(|ui| {
            let mut pick: Option<(&'static str, String)> = None;
            ui.label(RichText::new(tr(l, "Piste audio")).strong());
            for t in self.tracks.iter().filter(|t| t.audio) {
                if ui.selectable_label(t.selected, &t.label).clicked() {
                    pick = Some(("aid", t.id.to_string()));
                    ui.close();
                }
            }
            ui.separator();
            ui.label(RichText::new(tr(l, "Sous-titres")).strong());
            let any = self.tracks.iter().any(|t| !t.audio && t.selected);
            if ui.selectable_label(!any, tr(l, "Désactivés")).clicked() {
                pick = Some(("sid", "no".into()));
                ui.close();
            }
            for t in self.tracks.iter().filter(|t| !t.audio) {
                if ui.selectable_label(t.selected, &t.label).clicked() {
                    pick = Some(("sid", t.id.to_string()));
                    ui.close();
                }
            }
            if ui.button(tr(l, "Charger un sous-titre…")).clicked() {
                if let Some(f) = rfd::FileDialog::new().add_filter("Sous-titres", &["srt", "ass", "ssa", "vtt", "sub"]).pick_file() {
                    self.cmd(&["sub-add", &f.to_string_lossy(), "select"]);
                    self.refresh_tracks();
                }
                ui.close();
            }
            if let Some((k, v)) = pick {
                self.prop(k, &v);
                self.refresh_tracks();
            }
        });
        resp.on_hover_text(format!("{} / {}", tr(l, "Piste audio"), tr(l, "Sous-titres")));
    }

    fn playlist_panel(&mut self, ui: &mut egui::Ui) {
        let l = self.l;
        ui.horizontal(|ui| {
            ui.strong(tr(l, "Liste de lecture"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button(tr(l, "Vider la liste")).clicked() {
                    self.pl.clear();
                }
                if icons::button(ui, Icon::Plus, 12.0, false, tr(l, "Ajouter à la liste…")).clicked() {
                    self.open_dialog(true);
                }
            });
        });
        ui.separator();
        if self.pl.items.is_empty() {
            ui.weak(tr(l, "La liste est vide. Glissez des fichiers ici."));
            return;
        }
        let (mut go, mut del) = (None, None);
        egui::ScrollArea::vertical().id_salt("pl").auto_shrink([false, false]).show(ui, |ui| {
            for (i, p) in self.pl.items.iter().enumerate() {
                let name = p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                let icon = match kind_of(p) {
                    Some(MediaKind::Video) => Icon::Film,
                    Some(MediaKind::Audio) => Icon::Note,
                    _ => Icon::Picture,
                };
                ui.horizontal(|ui| {
                    if icons::button(ui, Icon::Close, 9.0, false, "").clicked() {
                        del = Some(i);
                    }
                    let cur = self.pl.cur == Some(i);
                    let (ir, _) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
                    icons::paint(ui.painter(), ir, icon, Color32::from_gray(170));
                    let t = RichText::new(name);
                    if ui.selectable_label(cur, if cur { t.color(accent()).strong() } else { t }).clicked() {
                        go = Some(i);
                    }
                });
            }
        });
        if let Some(i) = del {
            self.pl.remove(i);
        }
        if let Some(i) = go {
            self.play(i);
        }
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        let l = self.l;
        let mut open = self.show_settings;
        egui::Window::new(tr(l, "Réglages")).open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
            ff_kit::help::links(ui, "player", &[("Cinéma", "cinema"), ("Derushage", "derushage"), ("Pipette", "pipette"), ("Résidence", "residence"), ("Association", "association")]);
            ui.horizontal(|ui| {
                ui.label(tr(l, "Langue :"));
                for (g, name) in [(Lang::Fr, "Français"), (Lang::En, "English")] {
                    if ui.selectable_label(self.set.lang == g, name).clicked() {
                        self.set.lang = g;
                        self.l = g;
                    }
                }
            });
            ui.separator();
            ui.label(RichText::new(tr(l, "Thème (partagé avec MediaForge)")).strong());
            let mut sh = ff_profiles::load();
            let mut chg = false;
            ui.horizontal(|ui| {
                if icons::button(ui, Icon::Sun, 18.0, !sh.dark, tr(l, "Clair")).clicked() && sh.dark {
                    sh.dark = false;
                    chg = true;
                }
                if icons::button(ui, Icon::Moon, 18.0, sh.dark, tr(l, "Sombre")).clicked() && !sh.dark {
                    sh.dark = true;
                    chg = true;
                }
            });
            let mut id = sh.theme.clone();
            if ff_kit::theme::style_picker(ui, &mut id, sh.dark, l == Lang::En) {
                sh.theme = id;
                chg = true;
            }
            if chg {
                let _ = ff_profiles::save(&sh);
                ff_kit::theme::apply(ctx, ff_kit::theme::Style::from_id(&sh.theme), sh.dark);
                self.theme_seen = (sh.theme.clone(), sh.dark);
            }
            ui.separator();
            ui.checkbox(&mut self.set.load_siblings, tr(l, "Charger les fichiers du dossier à l'ouverture"));
            ui.checkbox(&mut self.set.resume, tr(l, "Reprendre la lecture où elle s'était arrêtée"));
            ui.separator();
            ui.label(RichText::new(tr(l, "Derushage")).strong());
            ui.checkbox(&mut self.set.tc_on, tr(l, "Timecode en surimpression (T)"));
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut self.set.tc_smpte, "SMPTE HH:MM:SS:FF");
                ui.checkbox(&mut self.set.tc_drop, "Drop-frame ;FF");
                ui.checkbox(&mut self.set.tc_frames, tr(l, "N° d'image"));
                ui.checkbox(&mut self.set.tc_ms, "HH:MM:SS.mmm");
                ui.checkbox(&mut self.set.tc_remain, tr(l, "Temps restant"));
            });
            ui.horizontal(|ui| {
                ui.label(tr(l, "Position"));
                for (i, n) in ["↖", "↗", "↙", "↘"].iter().enumerate() {
                    ui.selectable_value(&mut self.set.tc_corner, i as u8, *n);
                }
                ui.label(tr(l, "Taille"));
                ui.add(egui::Slider::new(&mut self.set.tc_size, 12.0..=60.0));
            });
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.set.notes_show, tr(l, "Afficher les notes sur l'image"));
                ui.label(tr(l, "Durée par défaut (s)"));
                ui.add(egui::DragValue::new(&mut self.set.notes_dur).range(0.5..=600.0).speed(0.1));
            });
            ui.separator();
            ui.checkbox(&mut self.set.cinema, tr(l, "Mode cinéma : bords flous en miroir si le format n'est pas celui de l'écran (touche C)"));
            ui.horizontal(|ui| {
                ui.label(tr(l, "Flou des bords (0 = net)"));
                ui.add(egui::Slider::new(&mut self.set.cinema_blur, 0..=6));
            });
            ui.checkbox(&mut self.set.hwdec, tr(l, "Accélération matérielle du décodage"));
            ui.checkbox(&mut self.set.hq_scaling, tr(l, "Mise à l'échelle haute qualité (GPU requis, au prochain démarrage)"));
            ui.horizontal(|ui| {
                ui.label(tr(l, "Durée du diaporama :"));
                ui.add(egui::DragValue::new(&mut self.set.slideshow_secs).range(1..=60).suffix(format!(" {}", tr(l, "s"))));
            });
            ui.horizontal(|ui| {
                ui.label(tr(l, "Pipette (touche I) : format copié au clic"));
                for (i, n) in ["#RRGGBB", "rgb(r, g, b)", "#RRGGBB + rgb"].iter().enumerate() {
                    ui.radio_value(&mut self.set.pick_fmt, i as u8, *n);
                }
            });
            ui.checkbox(&mut self.set.keep_resident, tr(l, "Rester en mémoire à la fermeture (réduit ; Ctrl+Q pour quitter)"));
            if crate::shell::SUPPORTED {
                if ui.checkbox(&mut self.set.autostart, tr(l, "Démarrer avec Windows (réduit)")).changed() {
                    if let Ok(exe) = std::env::current_exe() {
                        if let Err(e) = crate::shell::set_autostart(&exe, self.set.autostart) {
                            self.say(e.to_string());
                        }
                    }
                }
                ui.separator();
                if ui.button(tr(l, "Associer les formats… (Windows)")).clicked() {
                    if let Ok(exe) = std::env::current_exe() {
                        if let Err(e) = crate::shell::register(&exe) {
                            self.say(e.to_string());
                        }
                    }
                }
                ui.weak(tr(l, "Ouvre ensuite les « Applications par défaut » de Windows pour choisir MediaForge Player."));
            }
            ui.separator();
            ui.collapsing(tr(l, "Raccourcis"), |ui| {
                ui.label(tr(l, "Espace : lecture/pause · ←/→ : ±5 s (Ctrl : ±30 s) · ↑/↓ : volume · M : muet · F ou F11 : plein écran · PgSuiv/PgPréc ou N/P : suivant/précédent · [ ] : vitesse · S : capture · R : pivoter · + − 0 1 : zoom · L : liste · O : ouvrir · Échap : quitter le plein écran"));
            });
            ui.add_space(4.0);
            ui.label(RichText::new(format!("v{}", env!("CARGO_PKG_VERSION"))).small().weak());
        });
        if self.close_outside && self.outside.check(ctx, open) {
            open = false;
        }
        self.show_settings = open;
        if !open {
            save_settings(&self.set);
        }
    }

    fn missing_lib(&mut self, ui: &mut egui::Ui) {
        let l = self.l;
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() * 0.25);
            ui.heading(tr(l, "libmpv est introuvable."));
            ui.label(tr(l, "Placez libmpv-2.dll (Windows) ou libmpv.so.2 (Linux) à côté de l'exécutable, ou choisissez son dossier."));
            ui.weak(self.lib_err.clone().unwrap_or_default());
            ui.add_space(10.0);
            if ui.button(tr(l, "Choisir le dossier de libmpv…")).clicked() {
                if let Some(d) = rfd::FileDialog::new().pick_folder() {
                    self.set.mpv_dir = Some(d);
                    save_settings(&self.set);
                    // Relance : la bibliothèque est chargée au démarrage.
                    self.say(tr(l, "Redémarrez le lecteur."));
                }
            }
        });
    }
}

impl Drop for App {
    fn drop(&mut self) {
        // Enregistrement de direct en cours (ou en cours de démarrage) : ffmpeg est arrêté et le fichier finalisé.
        let pending = self.rec_slot.lock().ok().and_then(|mut g| g.take());
        for mut r in self.rec.take().into_iter().chain(pending) {
            let _ = r.stop();
        }
        self.remember();
        save_settings(&self.set);
        self.stop.store(true, Ordering::Relaxed);
    }
}

#[allow(deprecated)] // Id::new : pas encore de remplaçant stable pour des ids locaux
impl eframe::App for App {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = root.ctx().clone();
        let l = self.l;
        if self.lib_err.is_some() {
            egui::CentralPanel::default().show(root, |ui| self.missing_lib(ui));
            return;
        }
        if crate::START_MIN.swap(false, std::sync::atomic::Ordering::Relaxed) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        }
        self.pump(&ctx);
        if ctx.input(|i| i.key_pressed(egui::Key::F1)) {
            ff_kit::help::trigger(&ctx, "player", "lecture");
        }
        ff_kit::help::show_error(&ctx);
        // Résidence : la fermeture réduit la fenêtre (le lecteur reste en mémoire) ; Ctrl+Q quitte vraiment.
        if ctx.input(|i| i.viewport().close_requested()) && self.set.keep_resident && !self.quitting {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            if let Some(m) = &self.mpv {
                let _ = m.set_property("pause", "yes");
            }
        }
        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Q)) {
            self.quitting = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        // Dépôt de fichiers
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| Some(f.path().to_path_buf())).collect());
        if !dropped.is_empty() {
            let append = ctx.input(|i| i.modifiers.shift);
            self.open_paths(dropped, append);
        }
        self.keys(&ctx);

        // Vidéo : plein cadre, sous toute l'interface.
        if let Some(rc) = self.render.clone() {
            let dirty = self.dirty.clone();
            let rect = ctx.content_rect();
            let target = self.target.clone();
            // Mode cinéma : seulement pour une vraie vidéo, sans zoom utilisateur.
            let cin = if self.set.cinema && self.kind == Some(MediaKind::Video) && self.zoom == 0.0 {
                self.mpv.as_ref().and_then(|m| m.get_f64("video-out-params/aspect").or_else(|| m.get_f64("video-params/aspect"))).filter(|a| *a > 0.1).map(|a| (a * 1000.0) as u32).unwrap_or(0)
            } else {
                0
            };
            self.cinema_aspect.store(cin, std::sync::atomic::Ordering::Relaxed);
            let cin_a = self.cinema_aspect.clone();
            self.cinema_blur.store(self.set.cinema_blur, std::sync::atomic::Ordering::Relaxed);
            let blur_a = self.cinema_blur.clone();
            let probe = self.probe.clone();
            let cb = eframe::egui_glow::CallbackFn::new(move |info, painter| {
                if let (Ok(r), Ok(mut t)) = (rc.lock(), target.lock()) {
                    crate::video::draw(painter, info.screen_size_px, &r, &dirty, &mut t, cin_a.load(std::sync::atomic::Ordering::Relaxed) as f32 / 1000.0, blur_a.load(std::sync::atomic::Ordering::Relaxed), &probe);
                }
            });
            ctx.layer_painter(egui::LayerId::background()).add(egui::PaintCallback { rect, callback: Arc::new(cb) });
        }

        // Activité souris -> affichage des commandes
        let moved = ctx.input(|i| i.pointer.delta() != egui::Vec2::ZERO || i.pointer.any_pressed());
        if moved {
            self.last_move = Instant::now();
        }
        // Une barre masquée ne peut pas être survolée : sinon le curseur reste affiché en plein écran.
        // Test géométrique (zone des commandes / de la liste) : is_pointer_over_egui reste vrai sur toute la fenêtre
        // dès qu'une zone egui couvre la vidéo, ce qui empêchait la barre de se masquer.
        let over_bars = self.bars_shown
            && ctx.input(|i| {
                let sc = i.content_rect();
                i.pointer.latest_pos().map(|p| p.y > sc.bottom() - 96.0 || (self.show_list && p.x > sc.right() - 290.0)).unwrap_or(false)
            });
        let hovering_ui = over_bars || self.show_settings || self.show_notes || self.note_edit.is_some();
        // Plein écran : la barre disparaît toujours après inactivité (même en pause) ; elle revient au moindre mouvement.
        let visible = if self.fullscreen {
            self.pl.current().is_none() || hovering_ui || self.last_move.elapsed() < HIDE_AFTER
        } else {
            self.pl.current().is_none() || self.paused || self.last_move.elapsed() < HIDE_AFTER * 4 || hovering_ui
        };
        self.bars_shown = visible;
        // Curseur masqué en même temps que la barre (en plus de l'icône vide, plus fiable sous Windows).
        let hide_cursor = !visible && self.fullscreen;
        if hide_cursor != self.cursor_hidden {
            self.cursor_hidden = hide_cursor;
            ctx.send_viewport_cmd(egui::ViewportCommand::CursorVisible(!hide_cursor));
        }
        if !visible {
            ctx.set_cursor_icon(egui::CursorIcon::None);
        } else if self.last_move.elapsed() < HIDE_AFTER * 4 {
            ctx.request_repaint_after(Duration::from_millis(300));
        }

        let pp = ff_kit::theme::current().panel;
        let panel_fill = Color32::from_rgba_unmultiplied(pp.r(), pp.g(), pp.b(), 225);
        if visible {
            egui::Panel::bottom("controls").frame(egui::Frame::new().fill(panel_fill).inner_margin(egui::Margin::symmetric(10, 6))).show(root, |ui| self.controls(ui));
            if self.show_list {
                egui::Panel::right("playlist")
                    .default_size(280.0)
                    .frame(egui::Frame::new().fill(panel_fill).inner_margin(egui::Margin::same(8)))
                    .show(root, |ui| self.playlist_panel(ui));
            }
        }
        if self.show_settings {
            self.settings_window(&ctx);
        }
        self.notes_window(&ctx);
        self.note_dialog(&ctx);

        // Zone centrale : clics, molette, glisser
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(root, |ui| {
            let r = ui.interact(ui.max_rect(), egui::Id::new("video"), egui::Sense::click_and_drag());
            if self.pl.current().is_none() {
                ui.centered_and_justified(|ui| {
                    ui.label(RichText::new(tr(l, "Glissez un fichier audio, vidéo ou image ici\nou faites un clic droit pour ouvrir.")).size(18.0).color(Color32::GRAY));
                });
            } else if self.kind == Some(MediaKind::Audio) {
                let has_video = self.mpv.as_ref().and_then(|m| m.get_bool("current-tracks/video/albumart")).unwrap_or(false);
                if !has_video {
                    ui.vertical_centered(|ui| {
                        ui.add_space(ui.available_height() * 0.35);
                        let (ir, _) = ui.allocate_exact_size(egui::vec2(64.0, 64.0), egui::Sense::hover());
                        icons::paint(ui.painter(), ir, Icon::Note, Color32::from_gray(120));
                        let text = if self.meta.is_empty() {
                            self.pl.current().and_then(|p| p.file_stem()).map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
                        } else {
                            self.meta.clone()
                        };
                        ui.label(RichText::new(text).size(18.0).color(Color32::LIGHT_GRAY));
                    });
                }
            }
            if self.kind == Some(MediaKind::Audio) && self.pl.current().is_some() {
                let rc = ui.max_rect();
                let (w, h, gap) = (240.0f32.min(rc.width() - 40.0), 7.0, 5.0);
                let x0 = rc.center().x - w / 2.0;
                let y0 = rc.bottom() - 28.0 - 2.0 * h - gap;
                let pal = ff_kit::theme::current();
                for ch in 0..2 {
                    let y = y0 + ch as f32 * (h + gap);
                    let bar = egui::Rect::from_min_size(egui::pos2(x0, y), egui::vec2(w, h));
                    ui.painter().rect_filled(bar, 3.0, Color32::from_black_alpha(140));
                    let seg = 24;
                    for i in 0..seg {
                        let f = (i as f32 + 1.0) / seg as f32;
                        if f > self.vu[ch] + 0.001 {
                            break;
                        }
                        let col = if f > 0.9 { pal.err } else if f > 0.75 { Color32::from_rgb(0xfe, 0xcf, 0x23) } else { pal.ok };
                        let sw = w / seg as f32;
                        ui.painter().rect_filled(egui::Rect::from_min_size(egui::pos2(x0 + i as f32 * sw + 0.5, y), egui::vec2(sw - 1.0, h)), 1.0, col);
                    }
                    let px = x0 + w * self.vu_peak[ch];
                    ui.painter().rect_filled(egui::Rect::from_min_size(egui::pos2(px - 1.0, y), egui::vec2(2.0, h)), 0.0, Color32::WHITE);
                }
            }
            if let Some(mv) = &mut self.mesh {
                let rc = ui.max_rect();
                let ppp = ctx.pixels_per_point();
                let sz = [((rc.width() * ppp) as usize).clamp(64, 1400), ((rc.height() * ppp) as usize).clamp(64, 1000)];
                if r.dragged() {
                    let d = r.drag_delta();
                    mv.yaw += d.x * 0.01;
                    mv.pitch = (mv.pitch + d.y * 0.01).clamp(-1.55, 1.55);
                    mv.dirty = true;
                }
                let sc = ctx.input(|i| i.smooth_scroll_delta.y);
                if sc != 0.0 && r.hovered() {
                    mv.zoom = (mv.zoom * (1.0 + sc / 400.0)).clamp(0.1, 20.0);
                    mv.dirty = true;
                }
                if mv.size != sz {
                    mv.size = sz;
                    mv.dirty = true;
                }
                if mv.dirty || mv.tex.is_none() {
                    let img = crate::mesh::render(&mv.mesh, mv.yaw, mv.pitch, mv.zoom, sz[0], sz[1], ff_kit::theme::current().accent.gamma_multiply(1.0), mv.wire);
                    match &mut mv.tex {
                        Some(t) => t.set(img, egui::TextureOptions::LINEAR),
                        None => mv.tex = Some(ctx.load_texture("mesh", img, egui::TextureOptions::LINEAR)),
                    }
                    mv.dirty = false;
                }
                if let Some(t) = &mv.tex {
                    ui.painter().image(t.id(), rc, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                }
                ui.painter().text(rc.left_bottom() + egui::vec2(10.0, -10.0), egui::Align2::LEFT_BOTTOM, format!("{} · {} {}", tr(l, "Glisser : tourner · Molette : zoom · W : fil de fer"), mv.mesh.tris.len(), tr(l, "triangles")), egui::FontId::proportional(12.0), Color32::from_gray(150));
            }
            self.derush_overlay(ui);
            self.pick_ui(ui, &ctx, &r);
            if r.clicked() && !self.pick && !self.is_image() && self.pl.current().is_some() {
                self.toggle_pause();
            }
            if r.double_clicked() {
                self.set_fullscreen(&ctx, !self.fullscreen);
            }
            let scroll = ctx.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 && r.hovered() && self.mesh.is_none() {
                if self.is_image() {
                    self.apply_zoom(self.zoom + scroll as f64 / 240.0);
                } else {
                    self.set_volume(self.set.volume + scroll as f64 / 20.0);
                }
            }
            if r.dragged() && self.is_image() && self.mesh.is_none() {
                let d = r.drag_delta();
                let size = ctx.content_rect().size();
                let k = 2f64.powf(self.zoom);
                let cur = |m: &Mpv, p: &str| m.get_f64(p).unwrap_or(0.0);
                if let Some(m) = &self.mpv {
                    let _ = m.set_property("video-pan-x", &(cur(m, "video-pan-x") + d.x as f64 / (size.x as f64 * k)).to_string());
                    let _ = m.set_property("video-pan-y", &(cur(m, "video-pan-y") + d.y as f64 / (size.y as f64 * k)).to_string());
                }
            }
            r.context_menu(|ui| {
                if ui.button(tr(l, "Ouvrir…")).clicked() {
                    self.open_dialog(false);
                    ui.close();
                }
                if ui.button(tr(l, "Ajouter à la liste…")).clicked() {
                    self.open_dialog(true);
                    ui.close();
                }
                if self.pl.current().is_some() {
                    ui.separator();
                    if self.mesh.is_none() && ui.checkbox(&mut self.pick, tr(l, "Pipette de couleur (I)")).changed() {
                        ui.close();
                    }
                    if ui.button(tr(l, "Convertir avec MediaForge")).clicked() {
                        self.convert_current();
                        ui.close();
                    }
                    if ui.button(tr(l, "Afficher dans le dossier")).clicked() {
                        self.reveal();
                        ui.close();
                    }
                    if ui.button(tr(l, "Copier le chemin")).clicked() {
                        if let Some(p) = self.pl.current() {
                            ui.ctx().copy_text(p.display().to_string());
                        }
                        ui.close();
                    }
                    if self.is_image() && ui.button(tr(l, "Copier l'image")).clicked() {
                        self.copy_image();
                        ui.close();
                    }
                }
                ui.separator();
                if ui.button(tr(l, "Réglages")).clicked() {
                    self.show_settings = true;
                    ui.close();
                }
            });
        });

        // Titre discret + notification
        // Pas de titre en plein écran ; sinon sur une ligne, plus grand et lisible
        if visible && !self.fullscreen && self.pl.current().is_some() {
            let name = self.pl.current().and_then(|p| p.file_name()).map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            egui::Area::new(egui::Id::new("title")).anchor(egui::Align2::LEFT_TOP, [10.0, 8.0]).interactable(false).show(&ctx, |ui| {
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                egui::Frame::new().fill(Color32::from_black_alpha(170)).corner_radius(6.0).inner_margin(egui::Margin::symmetric(8, 4)).show(ui, |ui| {
                    ui.label(RichText::new(name).size(15.0).color(Color32::WHITE));
                });
            });
        }
        let newrec = self.rec_slot.lock().unwrap().take();
        if newrec.is_some() {
            self.rec = newrec;
        }
        if let Some(r) = &self.rec {
            let e = r.started.elapsed().as_secs();
            egui::Area::new(egui::Id::new("rec")).anchor(egui::Align2::RIGHT_TOP, [-10.0, 8.0]).interactable(false).show(&ctx, |ui| {
                ui.label(RichText::new(format!("● REC {}:{:02}", e / 60, e % 60)).color(Color32::from_rgb(0xff, 0x40, 0x40)).strong().background_color(Color32::from_black_alpha(140)));
            });
            ctx.request_repaint_after(Duration::from_millis(500));
        }
        let done = self.save_msg.lock().unwrap().take();
        if let Some(m) = done {
            self.say(m);
        }
        if let Some((t, at)) = &self.toast {
            if at.elapsed() < Duration::from_secs(3) {
                egui::Area::new(egui::Id::new("toast")).anchor(egui::Align2::CENTER_TOP, [0.0, 40.0]).interactable(false).show(&ctx, |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| ui.label(t));
                });
                ctx.request_repaint_after(Duration::from_millis(500));
            } else {
                self.toast = None;
            }
        }
    }
}
