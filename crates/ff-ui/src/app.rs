//! Application egui : file d'attente, profils, retouches, outils, réglages, journal. v0.3.0
#[allow(unused_imports)]
use crate::look::{accent, green, red};
use ff_kit::icons::{self, Icon};
use crate::i18n::{tr, trf};
use crate::util::{expand_dropped, fmt_secs, reveal, MEDIA_EXT};
use eframe::egui::{self, Color32, RichText};
use ff_core::{default_output, slug, split_args, unique_output, Edits, Event, HwAccel, Job, JobId, JobKind, Lang, Profile, Progress};
use ff_engine::{Binaries, Queue};
use ff_profiles::Settings;
use std::path::PathBuf;
use std::net::TcpListener;
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::time::Instant;

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, PartialEq)]
pub(crate) enum State {
    Idle,
    Queued,
    Running,
    Done,
    Failed(String),
    Cancelled,
}

impl State {
    pub(crate) fn is_final(&self) -> bool {
        matches!(self, State::Done | State::Failed(_) | State::Cancelled)
    }
    pub(crate) fn is_active(&self) -> bool {
        matches!(self, State::Queued | State::Running)
    }
}

pub(crate) struct Item {
    pub uid: u64,
    pub engine_id: Option<JobId>,
    pub input: PathBuf,
    pub profile: Profile,
    pub output: Option<PathBuf>,
    pub state: State,
    pub progress: Progress,
    pub kind: JobKind,
    pub edits: Edits,
    /// Titre affiché à la place du nom de fichier (outils : « Fusion de 3 fichiers »).
    pub title: Option<String>,
    /// Sortie imposée (outils) : non recalculée au lancement.
    pub fixed_output: bool,
    /// Nom de sortie choisi par l'utilisateur (sans extension) ; vide = nom du fichier source.
    pub out_name: String,
}

/// Brouillon d'édition d'un profil utilisateur.
#[derive(Clone, Default)]
struct Draft {
    id: String,
    label: String,
    ext: String,
    args: String,
    builtin: bool,
    is_new: bool,
    /// Éditeur visuel : modèle structuré (None = arguments bruts).
    spec: Option<ff_core::spec::VideoSpec>,
}

enum Action {
    Remove(usize),
    Cancel(usize),
    /// Remet une tâche annulée ou en échec « en attente » pour la relancer.
    Reset(usize),
    Reveal(usize),
    Edit(usize),
}

pub struct App {
    pub(crate) settings: Settings,
    pub(crate) profiles: Vec<Profile>,
    pub(crate) selected: usize,
    pub(crate) items: Vec<Item>,
    pub(crate) next_uid: u64,
    pub(crate) bins: Option<Binaries>,
    bins_err: Option<String>,
    ffmpeg_version: String,
    hw_avail: Vec<HwAccel>,
    engine: Option<(Queue, Receiver<Event>)>,
    ctx: egui::Context,
    started: Instant,
    log_lines: Vec<String>,
    show_settings: bool,
    outside: ff_kit::outside::OutsideClose,
    show_profiles: bool,
    show_log: bool,
    pub(crate) show_tools: bool,
    pub(crate) show_online: bool,
    pub(crate) online: crate::online::OnlineState,
    draft: Option<Draft>,
    draft_msg: String,
    pub(crate) edit: Option<crate::edit::EditDraft>,
    pub(crate) tools: crate::tools::ToolsState,
    inbox: Option<Receiver<Vec<PathBuf>>>,
    shell_msg: String,
    /// FFmpeg full : binaires trouvés, version active, session forcée en full, téléchargement et invite.
    full_bins: Option<Binaries>,
    using_full: bool,
    session_full: bool,
    full_dl: Option<Receiver<Result<PathBuf, String>>>,
    full_msg: String,
    full_prompt: bool,
    full_then_start: bool,
    full_declined: bool,
    /// Éditeur visuel : encodeurs disponibles et aperçu de l'image.
    encoders: std::collections::HashSet<String>,
    prev_tex: Option<egui::TextureHandle>,
    prev_rx: Option<Receiver<Result<(usize, usize, Vec<u8>), String>>>,
    prev_msg: String,
    prev_src: Option<PathBuf>,
    prev_at: f64,
}

impl Item {
    pub(crate) fn display_name(&self) -> String {
        self.title.clone().unwrap_or_else(|| self.input.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
    }
}

/// Libellé d'un profil : traduit pour les profils intégrés, tel quel pour les profils utilisateur.
pub(crate) fn plabel(l: Lang, p: &Profile) -> String {
    if p.builtin { tr(l, &p.label).to_string() } else { p.label.clone() }
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, listener: Option<TcpListener>) -> Self {
        let settings = ff_profiles::load();
        let profiles = settings.all_profiles();
        let selected = profiles.iter().position(|p| p.id == settings.last_profile).unwrap_or(0);
        let mut app = Self {
            settings,
            profiles,
            selected,
            items: Vec::new(),
            next_uid: 1,
            bins: None,
            bins_err: None,
            ffmpeg_version: String::new(),
            hw_avail: vec![HwAccel::None],
            engine: None,
            ctx: cc.egui_ctx.clone(),
            started: Instant::now(),
            log_lines: Vec::new(),
            show_settings: false,
            full_bins: None,
            using_full: false,
            session_full: false,
            full_dl: None,
            full_msg: String::new(),
            full_prompt: false,
            full_then_start: false,
            full_declined: false,
            encoders: Default::default(),
            prev_tex: None,
            prev_rx: None,
            prev_msg: String::new(),
            prev_src: None,
            prev_at: 1.0,
            outside: Default::default(),
            show_profiles: false,
            show_log: false,
            show_tools: false,
            show_online: false,
            online: Default::default(),
            draft: None,
            draft_msg: String::new(),
            edit: None,
            tools: Default::default(),
            inbox: None,
            shell_msg: String::new(),
        };
        if let Some(l) = listener {
            let ctx = cc.egui_ctx.clone();
            app.inbox = Some(ff_core::instance::serve(l, move || {
                // Fenêtre réduite : sans restauration, aucune image n'est dessinée et le lot reste en attente.
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                ctx.request_repaint();
            }));
        }
        app.apply_theme();
        app.init_engine();
        app
    }

    // ---------- moteur ----------

    /// (Re)localise FFmpeg et recrée la file. Les tâches actives sont marquées annulées.
    fn init_engine(&mut self) {
        for it in self.items.iter_mut().filter(|i| i.state.is_active()) {
            it.state = State::Cancelled;
        }
        self.engine = None;
        // Version full (GPU) si elle est demandée (réglage ou session) et installée ; sinon lite, puis full en secours.
        self.full_bins = ff_online::ffbuild::locate_full(self.settings.ffmpeg_full_dir.as_deref()).and_then(|d| ff_engine::locate(Some(&d)).ok());
        let want_full = self.settings.ffmpeg_flavor == "full" || self.session_full;
        let located = match (want_full, &self.full_bins) {
            (true, Some(f)) => Ok(f.clone()),
            _ => ff_engine::locate(self.settings.ffmpeg_dir.as_deref()).or_else(|e| self.full_bins.clone().ok_or(e)),
        };
        self.using_full = matches!((&located, &self.full_bins), (Ok(b), Some(f)) if b.ffmpeg == f.ffmpeg);
        match located {
            Ok(b) => {
                self.ffmpeg_version = ff_engine::version(&b).unwrap_or_default();
                self.hw_avail = ff_engine::available_hw(&b);
                self.encoders = ff_engine::available_encoders(&b);
                if self.settings.hw != HwAccel::Auto && !self.hw_avail.contains(&self.settings.hw) {
                    self.settings.hw = HwAccel::None;
                }
                let ctx = self.ctx.clone();
                let notify: Arc<dyn Fn() + Send + Sync> = Arc::new(move || ctx.request_repaint());
                self.engine = Some(Queue::new(b.clone(), self.settings.parallel, notify));
                self.bins = Some(b);
                self.bins_err = None;
            }
            Err(e) => {
                self.bins = None;
                self.bins_err = Some(e.to_string());
            }
        }
    }

    fn poll_events(&mut self) {
        let mut evs = Vec::new();
        if let Some((_, rx)) = &self.engine {
            while let Ok(e) = rx.try_recv() {
                evs.push(e);
            }
        }
        for e in evs {
            let id = match &e {
                Event::Started(i) | Event::Progress(i, _) | Event::Done(i) | Event::Failed(i, _) | Event::Cancelled(i) | Event::Info(i, _) => *i,
            };
            let Some(pos) = self.items.iter().position(|it| it.engine_id == Some(id)) else { continue };
            let name = self.items[pos].display_name();
            match e {
                Event::Started(_) => self.items[pos].state = State::Running,
                Event::Progress(_, p) => self.items[pos].progress = p,
                Event::Done(_) => {
                    self.items[pos].state = State::Done;
                    self.items[pos].progress.percent = 100.0;
                }
                Event::Cancelled(_) => self.items[pos].state = State::Cancelled,
                Event::Info(_, msg) => self.log(&format!("INFO {name} : {msg}")),
                Event::Failed(_, msg) => {
                    self.log(&format!("ERREUR {name} : {msg}"));
                    self.items[pos].state = State::Failed(msg);
                }
            }
        }
    }

    fn log(&mut self, msg: &str) {
        let t = self.started.elapsed().as_secs();
        self.log_lines.push(format!("[+{:02}:{:02}] {msg}", t / 60, t % 60));
        ff_profiles::log(msg);
    }

    // ---------- état ----------

    pub(crate) fn apply_theme(&self) {
        let style = ff_kit::theme::Style::from_id(&self.settings.theme);
        crate::look::set(ff_kit::theme::palette(style, self.settings.dark));
        ff_kit::theme::apply(&self.ctx, style, self.settings.dark);
    }

    pub(crate) fn save_settings(&mut self) {
        if let Err(e) = ff_profiles::save(&self.settings) {
            self.log(&format!("Enregistrement des réglages impossible : {e:#}"));
        }
    }

    fn refresh_profiles(&mut self) {
        self.profiles = self.settings.all_profiles();
        self.selected = self.profiles.iter().position(|p| p.id == self.settings.last_profile).unwrap_or(0);
    }

    /// Profil à appliquer à un fichier : le profil choisi s'il convient au type du fichier, sinon le premier qui convient.
    pub(crate) fn profile_for(&self, input: &std::path::Path) -> Profile {
        let chosen = self.profiles.get(self.selected).cloned().unwrap_or_else(|| ff_core::builtin_profiles().remove(0));
        match ff_core::media::kind_of(input) {
            Some(k) if !ff_core::media::profile_accepts(&chosen, k) => {
                self.profiles.iter().find(|p| ff_core::media::profile_accepts(p, k)).cloned().unwrap_or(chosen)
            }
            _ => chosen,
        }
    }

    pub(crate) fn add_files(&mut self, files: Vec<PathBuf>) {
        for f in files {
            if self.items.iter().any(|i| i.input == f && !i.state.is_final()) {
                continue; // déjà dans la liste
            }
            let profile = self.profile_for(&f);
            self.items.push(Item {
                uid: self.next_uid,
                engine_id: None,
                input: f,
                profile,
                output: None,
                state: State::Idle,
                progress: Progress::default(),
                kind: JobKind::Convert,
                edits: Edits::default(),
                title: None,
                fixed_output: false,
                out_name: String::new(),
            });
            self.next_uid += 1;
        }
    }

    /// Sortie unique, y compris face aux sorties déjà réservées par d'autres éléments.
    fn reserve_output(&self, input: &PathBuf, profile: &Profile, out_name: &str) -> PathBuf {
        let mut base = default_output(input, self.settings.output_dir.as_deref(), &profile.extension);
        let name = out_name.trim();
        if !name.is_empty() {
            base.set_file_name(format!("{name}.{}", profile.extension));
        }
        let mut out = unique_output(base, input);
        let taken = |p: &PathBuf| self.items.iter().any(|i| !i.state.is_final() && i.output.as_ref() == Some(p));
        let mut n = 1;
        while taken(&out) && n < 10_000 {
            let stem = out.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let base = stem.rsplit_once(" (").map(|(b, _)| b.to_string()).unwrap_or(stem);
            out.set_file_name(format!("{base} ({n}).{}", profile.extension));
            out = unique_output(out, input);
            n += 1;
        }
        out
    }

    /// Réinitialise une tâche annulée / en échec : elle repart de zéro à la prochaine conversion.
    fn reset_item(&mut self, i: usize) {
        let it = &mut self.items[i];
        if matches!(it.state, State::Cancelled | State::Failed(_)) {
            it.state = State::Idle;
            it.progress = Progress::default();
            it.engine_id = None;
            if !it.fixed_output {
                it.output = None;
            }
        }
    }

    fn start_all(&mut self) {
        let Some(_) = &self.engine else { return };
        // Version lite sans encodeur GPU alors qu'une accélération est demandée : proposer la version full.
        let gpu_wanted = self.settings.hw != HwAccel::None;
        let lite_no_gpu = self.hw_avail.iter().all(|h| *h == HwAccel::None);
        if !self.using_full && gpu_wanted && lite_no_gpu && self.settings.ffmpeg_ask_full && !self.full_declined && !self.items.iter().any(|i| i.state.is_active()) {
            self.full_prompt = true;
            return;
        }
        self.full_declined = false;
        for i in 0..self.items.len() {
            if self.items[i].state != State::Idle {
                continue;
            }
            let out = match (&self.items[i].output, self.items[i].fixed_output) {
                (Some(o), true) => o.clone(),
                _ => self.reserve_output(&self.items[i].input.clone(), &self.items[i].profile.clone(), &self.items[i].out_name.clone()),
            };
            if let Some(dir) = out.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            self.items[i].output = Some(out.clone());
            let it = &self.items[i];
            let mut job = Job::convert(it.input.clone(), out, it.profile.clone());
            job.kind = it.kind.clone();
            job.edits = it.edits.clone();
            job.hw = self.settings.hw.resolve(&self.hw_avail);
            let id = self.engine.as_ref().unwrap().0.enqueue(job);
            self.items[i].engine_id = Some(id);
            self.items[i].state = State::Queued;
        }
    }

    /// Lance le téléchargement de FFmpeg full ; `then_start` : relance la conversion une fois installé.
    fn download_full(&mut self, then_start: bool) {
        if self.full_dl.is_some() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(ff_online::ffbuild::install_full());
            ctx.request_repaint();
        });
        self.full_dl = Some(rx);
        self.full_then_start = then_start;
        self.full_msg = tr(self.settings.lang, "Téléchargement de FFmpeg full en cours (environ 150 Mo)…").to_string();
    }

    /// Résultat du téléchargement : bascule sur la version full pour la session et relance si demandé.
    fn poll_full(&mut self) {
        let Some(rx) = &self.full_dl else { return };
        let Ok(r) = rx.try_recv() else { return };
        self.full_dl = None;
        let l = self.settings.lang;
        match r {
            Ok(_) => {
                self.full_msg = tr(l, "FFmpeg full installé.").to_string();
                if self.full_then_start && !self.items.iter().any(|i| i.state.is_active()) {
                    self.session_full = true;
                    self.init_engine();
                    self.start_all();
                } else {
                    self.init_engine();
                }
            }
            Err(e) => {
                self.full_msg = format!("{} {e}", tr(l, "Échec du téléchargement :"));
                self.log(&self.full_msg.clone());
            }
        }
        self.full_then_start = false;
    }

    /// Invite : utiliser la version full (GPU) pour cet encodage ?
    fn full_prompt_window(&mut self, ctx: &egui::Context) {
        let l = self.settings.lang;
        let mut act: Option<u8> = None;
        egui::Window::new(tr(l, "FFmpeg full (GPU)")).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
            ff_kit::help::button(ui, "mediaforge", "ffmpeg-lite-full", "Aide");
            ui.label(tr(l, "La version « lite » de FFmpeg n'encode pas avec le GPU : la conversion utiliserait le processeur."));
            ui.label(tr(l, "Utiliser la version « full » (NVENC, AMF, Quick Sync) pour cet encodage ?"));
            let installed = self.full_bins.is_some();
            if !installed {
                ui.label(RichText::new(tr(l, "Elle n'est pas installée : elle sera téléchargée (environ 150 Mo).")).small().weak());
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let lbl = if installed { tr(l, "Utiliser la version full") } else { tr(l, "Télécharger et utiliser la version full") };
                if ui.button(lbl).clicked() {
                    act = Some(1);
                }
                if ui.button(tr(l, "Continuer en lite (CPU)")).clicked() {
                    act = Some(2);
                }
                if ui.button(tr(l, "Annuler")).clicked() {
                    act = Some(0);
                }
            });
            let mut ask = self.settings.ffmpeg_ask_full;
            if ui.checkbox(&mut ask, tr(l, "Me le proposer à chaque fois")).changed() {
                self.settings.ffmpeg_ask_full = ask;
                self.save_settings();
            }
        });
        match act {
            Some(1) => {
                self.full_prompt = false;
                if self.full_bins.is_some() {
                    self.session_full = true;
                    self.init_engine();
                    self.start_all();
                } else {
                    self.download_full(true);
                }
            }
            Some(2) => {
                self.full_prompt = false;
                self.full_declined = true;
                self.start_all();
            }
            Some(_) => self.full_prompt = false,
            None => {}
        }
    }

    // ---------- fenêtres ----------

    fn settings_window(&mut self, ctx: &egui::Context) {
        let l = self.settings.lang;
        let mut open = self.show_settings;
        egui::Window::new(tr(l, "Réglages")).open(&mut open).resizable(false).show(ctx, |ui| {
            ff_kit::help::links(ui, "mediaforge", &[("GPU", "gpu"), ("FFmpeg lite / full", "ffmpeg-lite-full"), ("Menu contextuel", "menu-contextuel"), ("Thèmes", "themes")]);
            ui.horizontal(|ui| {
                ui.label(tr(l, "Langue :"));
                for (code, name) in [(Lang::Fr, "Français"), (Lang::En, "English")] {
                    if ui.selectable_label(self.settings.lang == code, name).clicked() && self.settings.lang != code {
                        self.settings.lang = code;
                        self.save_settings();
                    }
                }
            });
            ui.label(RichText::new(tr(l, "Thème :")).strong());
            ui.horizontal(|ui| {
                let mut changed = false;
                for (dark, ic, label) in [(false, Icon::Sun, "Clair"), (true, Icon::Moon, "Sombre")] {
                    if icons::labeled(ui, ic, tr(l, label), if self.settings.dark == dark { Some(accent()) } else { None }, true, "").clicked() && self.settings.dark != dark {
                        self.settings.dark = dark;
                        changed = true;
                    }
                }
                if changed {
                    self.apply_theme();
                    self.save_settings();
                }
            });
            let mut id = self.settings.theme.clone();
            if ff_kit::theme::style_picker(ui, &mut id, self.settings.dark, l == Lang::En) {
                self.settings.theme = id;
                self.apply_theme();
                self.save_settings();
            }
            ui.horizontal(|ui| {
                ui.label(tr(l, "Conversions simultanées :"));
                let mut n = self.settings.parallel;
                if ui.add(egui::Slider::new(&mut n, 1..=4)).changed() {
                    self.settings.parallel = n;
                    if let Some((q, _)) = &self.engine {
                        q.set_parallelism(n);
                    }
                    self.save_settings();
                }
            });
            ui.horizontal(|ui| {
                ui.label(tr(l, "Accélération matérielle :"));
                egui::ComboBox::from_id_salt("hw").selected_text(tr(l, self.settings.hw.label())).show_ui(ui, |ui| {
                    let mut opts = self.hw_avail.clone();
                    opts.insert(1.min(opts.len()), HwAccel::Auto);
                    for h in opts {
                        if ui.selectable_label(self.settings.hw == h, tr(l, h.label())).clicked() {
                            self.settings.hw = h;
                            self.save_settings();
                        }
                    }
                });
            });
            ui.label(RichText::new(tr(l, "En cas d'échec, la conversion est relancée automatiquement en logiciel.")).small().weak());
            ui.label(RichText::new(tr(l, "Seuls H.264 et H.265 sont accélérés par le GPU ; les autres codecs (VP9, AV1, MPEG-4…) restent sur le CPU.")).small().weak());
            ui.separator();
            ui.label("FFmpeg :");
            match (&self.bins, &self.bins_err) {
                (Some(b), _) => {
                    ui.label(RichText::new(b.ffmpeg.display().to_string()).small().weak());
                    ui.label(RichText::new(&self.ffmpeg_version).small().weak());
                }
                (_, Some(e)) => {
                    ui.colored_label(red(), tr(l, e));
                }
                _ => {}
            }
            ui.horizontal(|ui| {
                if ui.button(tr(l, "Choisir le dossier FFmpeg…")).clicked() {
                    if let Some(d) = rfd::FileDialog::new().pick_folder() {
                        self.settings.ffmpeg_dir = Some(d);
                        self.save_settings();
                        self.init_engine();
                    }
                }
                if ui.button(tr(l, "Détection automatique")).clicked() {
                    self.settings.ffmpeg_dir = None;
                    self.save_settings();
                    self.init_engine();
                }
            });
            ui.label(RichText::new(if self.using_full { tr(l, "Version active : full (GPU)") } else { tr(l, "Version active : lite") }).small());
            ui.horizontal(|ui| {
                ui.label(tr(l, "Version par défaut :"));
                for (id, n) in [("lite", "Lite"), ("full", "Full (GPU)")] {
                    if ui.radio(self.settings.ffmpeg_flavor == id, n).clicked() && self.settings.ffmpeg_flavor != id {
                        self.settings.ffmpeg_flavor = id.into();
                        self.session_full = false;
                        self.save_settings();
                        self.init_engine();
                    }
                }
            });
            let mut ask = self.settings.ffmpeg_ask_full;
            if ui.checkbox(&mut ask, tr(l, "Proposer la version full quand l'encodage GPU est demandé")).changed() {
                self.settings.ffmpeg_ask_full = ask;
                self.save_settings();
            }
            ui.horizontal(|ui| {
                let have = self.full_bins.is_some();
                ui.label(RichText::new(if have { tr(l, "FFmpeg full : installé") } else { tr(l, "FFmpeg full : non installé") }).small().weak());
                let lbl = if have { tr(l, "Mettre à jour full") } else { tr(l, "Télécharger FFmpeg full") };
                if ui.add_enabled(self.full_dl.is_none(), egui::Button::new(lbl)).clicked() {
                    self.download_full(false);
                }
                if ui.button(tr(l, "Choisir le dossier full…")).clicked() {
                    if let Some(d) = rfd::FileDialog::new().pick_folder() {
                        self.settings.ffmpeg_full_dir = Some(d);
                        self.save_settings();
                        self.init_engine();
                    }
                }
            });
            if !self.full_msg.is_empty() {
                ui.label(RichText::new(&self.full_msg).small().weak());
            }
            if crate::shell::SUPPORTED {
                ui.separator();
                ui.label(tr(l, "Menu contextuel de l'explorateur :"));
                ui.horizontal(|ui| {
                    if ui.button(tr(l, "Activer")).clicked() {
                        self.shell_msg = match std::env::current_exe().map_err(anyhow::Error::from).and_then(|e| crate::shell::install(&e, tr(l, "Convertir avec MediaForge"))) {
                            Ok(()) => tr(l, "Menu contextuel activé.").to_string(),
                            Err(e) => format!("{e:#}"),
                        };
                    }
                    if ui.button(tr(l, "Retirer")).clicked() {
                        self.shell_msg = match crate::shell::uninstall() {
                            Ok(()) => tr(l, "Menu contextuel retiré.").to_string(),
                            Err(e) => format!("{e:#}"),
                        };
                    }
                });
                if !self.shell_msg.is_empty() {
                    ui.label(RichText::new(&self.shell_msg).small());
                }
            }
            ui.separator();
            if ui.checkbox(&mut self.settings.close_on_outside, tr(l, "Fermer les fenêtres de réglages en cliquant à l'extérieur")).changed() {
                self.save_settings();
            }
            ui.label(RichText::new(format!("{} {}", tr(l, "Données :"), ff_profiles::data_dir().display())).small().weak());
        });
        if self.settings.close_on_outside && self.outside.check(ctx, open) {
            open = false;
        }
        self.show_settings = open;
    }

    fn log_window(&mut self, ctx: &egui::Context) {
        let l = self.settings.lang;
        let mut open = self.show_log;
        egui::Window::new(tr(l, "Journal")).open(&mut open).default_size([620.0, 300.0]).show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button(tr(l, "Effacer")).clicked() {
                    self.log_lines.clear();
                }
                if ui.button(tr(l, "Ouvrir le dossier du journal")).clicked() {
                    reveal(&ff_profiles::log_path());
                }
            });
            egui::ScrollArea::vertical().stick_to_bottom(true).show(ui, |ui| {
                if self.log_lines.is_empty() {
                    ui.label(RichText::new(tr(l, "Aucune erreur.")).weak());
                }
                for line in &self.log_lines {
                    ui.label(RichText::new(line).monospace().small());
                }
            });
        });
        self.show_log = open;
    }

    fn profiles_window(&mut self, ctx: &egui::Context) {
        let l = self.settings.lang;
        let mut open = self.show_profiles;
        let mut to_select: Option<Draft> = None;
        let mut save: Option<Draft> = None;
        let mut delete: Option<String> = None;
        let mut preview_req: Option<ff_core::spec::VideoSpec> = None;
        egui::Window::new(tr(l, "Profils d'encodage")).open(&mut open).default_size([980.0, 660.0]).show(ctx, |ui| {
            ff_kit::help::links(ui, "mediaforge", &[("Éditeur visuel", "profils-visuels"), ("Incrustations", "incrustations"), ("Aperçu", "apercu"), ("Mode avancé", "profils-avances")]);
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(230.0);
                    if ui.button(tr(l, "+ Nouveau profil")).clicked() {
                        to_select = Some(Draft {
                            id: String::new(),
                            label: tr(l, "Mon profil").into(),
                            ext: "mp4".into(),
                            args: "-c:v libx264 -crf 23 -c:a aac -b:a 128k".into(),
                            builtin: false,
                            is_new: true,
                            spec: Some(ff_core::spec::VideoSpec::default()),
                        });
                    }
                    egui::ScrollArea::vertical().id_salt("plist").max_height(340.0).show(ui, |ui| {
                        for p in &self.profiles {
                            let sel = self.draft.as_ref().map(|d| !d.is_new && d.id == p.id).unwrap_or(false);
                            let text = if p.builtin { plabel(l, p) } else { format!("{} {}", p.label, tr(l, "(perso)")) };
                            if ui.selectable_label(sel, text).clicked() {
                                to_select = Some(Draft {
                                    id: p.id.clone(),
                                    label: p.label.clone(),
                                    ext: p.extension.clone(),
                                    args: p.args.iter().map(|a| if a.contains(' ') { format!("\"{a}\"") } else { a.clone() }).collect::<Vec<_>>().join(" "),
                                    builtin: p.builtin,
                                    is_new: false,
                                    spec: p.spec.clone(),
                                });
                            }
                        }
                    });
                });
                ui.separator();
                ui.vertical(|ui| {
                    let Some(d) = self.draft.as_mut() else {
                        ui.label(RichText::new(tr(l, "Sélectionnez un profil ou créez-en un.")).weak());
                        return;
                    };
                    let mut spec_changed = false;
                    let mut want_preview = false;
                    ui.add_enabled_ui(!d.builtin, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(tr(l, "Nom :"));
                            ui.add(egui::TextEdit::singleline(&mut d.label).desired_width(380.0));
                        });
                        if let Some(sp) = d.spec.as_mut() {
                            egui::ScrollArea::vertical().id_salt("spec_scroll").max_height(500.0).auto_shrink([false, false]).show(ui, |ui| {
                                spec_changed = crate::spec_ui::editor(ui, l, sp, &self.encoders);
                                ui.separator();
                                ui.label(RichText::new(tr(l, "Aperçu")).strong());
                                ui.horizontal(|ui| {
                                    if ui.button(tr(l, "Aperçu sur une vidéo…")).clicked() {
                                        if let Some(p) = rfd::FileDialog::new().add_filter("Vidéo", MEDIA_EXT).pick_file() {
                                            self.prev_src = Some(p);
                                            want_preview = true;
                                        }
                                    }
                                    if ui.add_enabled(self.prev_src.is_some(), egui::Button::new(tr(l, "Actualiser"))).clicked() {
                                        want_preview = true;
                                    }
                                    ui.label(tr(l, "Image à (s)"));
                                    ui.add(egui::DragValue::new(&mut self.prev_at).range(0.0..=36000.0).speed(0.5));
                                    if self.prev_rx.is_some() {
                                        ui.spinner();
                                    }
                                });
                                if let Some(s) = &self.prev_src {
                                    ui.label(RichText::new(s.display().to_string()).small().weak());
                                }
                                if !self.prev_msg.is_empty() {
                                    ui.colored_label(red(), &self.prev_msg);
                                }
                                if let Some(t) = &self.prev_tex {
                                    let w = ui.available_width().min(640.0);
                                    let sz = t.size_vec2();
                                    ui.add(egui::Image::new((t.id(), egui::vec2(w, w * sz.y / sz.x.max(1.0)))));
                                }
                            });
                        } else {
                            egui::Grid::new("pedit").num_columns(2).spacing([8.0, 8.0]).show(ui, |ui| {
                                ui.label(tr(l, "Extension :"));
                                ui.add(egui::TextEdit::singleline(&mut d.ext).desired_width(100.0));
                                ui.end_row();
                                ui.label(tr(l, "Arguments FFmpeg :"));
                                ui.add(egui::TextEdit::multiline(&mut d.args).desired_width(380.0).desired_rows(3));
                                ui.end_row();
                                ui.label(tr(l, "Modèle :"));
                                egui::ComboBox::from_id_salt("tpl").selected_text(tr(l, "Remplir depuis un modèle…")).show_ui(ui, |ui| {
                                    for (name, ext, a) in TEMPLATES {
                                        if ui.selectable_label(false, tr(l, name)).clicked() {
                                            d.ext = (*ext).into();
                                            d.args = (*a).into();
                                        }
                                    }
                                });
                                ui.end_row();
                            });
                            if ui.button(tr(l, "Utiliser l'éditeur visuel (remplace les arguments)")).clicked() {
                                d.spec = Some(ff_core::spec::VideoSpec::default());
                            }
                        }
                    });
                    if let Some(sp) = d.spec.as_ref() {
                        d.ext = sp.extension().to_string();
                        d.args = sp.to_args().iter().map(|a| if a.contains(' ') || a.contains('[') { format!("\"{a}\"") } else { a.clone() }).collect::<Vec<_>>().join(" ");
                    }
                    let _ = spec_changed;
                    if want_preview {
                        preview_req = d.spec.clone();
                    }
                    ui.add_space(6.0);
                    if d.spec.is_some() && !d.builtin {
                        if ui.small_button(tr(l, "Passer en mode avancé (arguments bruts)")).clicked() {
                            d.spec = None;
                        }
                    }
                    ui.label(RichText::new(format!("ffmpeg -i <{}> {} <{}>.{}", tr(l, "entrée"), d.args, tr(l, "sortie"), d.ext)).monospace().small().weak());
                    ui.add_space(6.0);
                    if d.builtin {
                        ui.label(RichText::new(tr(l, "Profil intégré : non modifiable.")).weak());
                        if ui.button(tr(l, "Dupliquer en profil personnalisé")).clicked() {
                            let mut n = d.clone();
                            n.builtin = false;
                            n.is_new = true;
                            n.label = format!("{} {}", tr(l, &n.label), tr(l, "(copie)"));
                            self.draft = Some(n);
                        }
                    } else {
                        ui.horizontal(|ui| {
                            if ui.add(egui::Button::new(RichText::new(tr(l, "Enregistrer")).color(Color32::WHITE)).fill(accent())).clicked() {
                                save = Some(d.clone());
                            }
                            if !d.is_new && ui.button(tr(l, "Supprimer")).clicked() {
                                delete = Some(d.id.clone());
                            }
                        });
                    }
                    if !self.draft_msg.is_empty() {
                        ui.colored_label(red(), &self.draft_msg);
                    }
                });
            });
        });
        self.show_profiles = open;
        if let Some(sp) = preview_req {
            self.start_preview(&sp);
        }
        self.poll_preview(ctx);
        if let Some(d) = to_select {
            self.draft = Some(d);
            self.draft_msg.clear();
        }
        if let Some(id) = delete {
            self.settings.custom_profiles.retain(|p| p.id != id);
            if self.settings.last_profile == id {
                self.settings.last_profile = "mp4".into();
            }
            self.draft = None;
            self.save_settings();
            self.refresh_profiles();
        }
        if let Some(d) = save {
            self.save_draft(d);
        }
    }

    /// Génère une image d'aperçu (1 image de la vidéo choisie avec les incrustations) dans un thread.
    fn start_preview(&mut self, sp: &ff_core::spec::VideoSpec) {
        let (Some(src), Some(b)) = (self.prev_src.clone(), self.bins.clone()) else {
            self.prev_msg = tr(self.settings.lang, "FFmpeg est introuvable.").into();
            return;
        };
        if self.prev_rx.is_some() {
            return;
        }
        self.prev_msg.clear();
        let fps = ff_probe::probe(&b.ffprobe, &src).ok().and_then(|i| i.fps).unwrap_or_else(|| "25".into());
        let args = sp.preview_args(&src.to_string_lossy(), self.prev_at, &fps);
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let mut c = std::process::Command::new(&b.ffmpeg);
            c.args(&args).stdin(std::process::Stdio::null());
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                c.creation_flags(0x0800_0000);
            }
            let r = match c.output() {
                Ok(o) if o.status.success() => parse_ppm(&o.stdout),
                Ok(o) => Err(String::from_utf8_lossy(&o.stderr).lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("échec de l'aperçu").to_string()),
                Err(e) => Err(e.to_string()),
            };
            let _ = tx.send(r);
            ctx.request_repaint();
        });
        self.prev_rx = Some(rx);
    }

    fn poll_preview(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.prev_rx else { return };
        let Ok(r) = rx.try_recv() else { return };
        self.prev_rx = None;
        match r {
            Ok((w, h, rgb)) => {
                let img = egui::ColorImage::from_rgb([w, h], &rgb);
                self.prev_tex = Some(ctx.load_texture("spec_preview", img, egui::TextureOptions::LINEAR));
            }
            Err(e) => self.prev_msg = e,
        }
    }

    fn save_draft(&mut self, d: Draft) {
        let l = self.settings.lang;
        let ext = d.ext.trim().trim_start_matches('.').to_ascii_lowercase();
        let args = match &d.spec {
            Some(sp) => sp.to_args(),
            None => split_args(&d.args),
        };
        let ext = if let Some(sp) = &d.spec { sp.extension().to_string() } else { ext };
        if d.label.trim().is_empty() {
            self.draft_msg = tr(l, "Le nom est obligatoire.").into();
        } else if ext.is_empty() || !ext.chars().all(|c| c.is_ascii_alphanumeric()) {
            self.draft_msg = tr(l, "Extension invalide (lettres et chiffres uniquement).").into();
        } else if args.is_empty() {
            self.draft_msg = tr(l, "Indiquez au moins un argument (ex. -c copy).").into();
        } else if let Some(p) = d.spec.as_ref().and_then(|sp| sp.problems().first().copied()) {
            self.draft_msg = tr(l, p).into();
        } else if d.spec.is_none() && args.iter().any(|a| a == "-i" || a == "-y" || a == "-n") {
            self.draft_msg = tr(l, "Les options -i, -y et -n sont gérées par l'application.").into();
        } else {
            self.draft_msg.clear();
            if d.is_new {
                let mut id = slug(&d.label);
                while self.profiles.iter().any(|p| p.id == id) {
                    id.push('x');
                }
                self.settings.custom_profiles.push(Profile { id: id.clone(), label: d.label.trim().into(), extension: ext, args, builtin: false, spec: d.spec.clone() });
                self.settings.last_profile = id.clone();
                self.draft = Some(Draft { id, is_new: false, ..d });
            } else if let Some(p) = self.settings.custom_profiles.iter_mut().find(|p| p.id == d.id) {
                p.label = d.label.trim().into();
                p.extension = ext;
                p.args = args;
                p.spec = d.spec.clone();
            }
            self.save_settings();
            self.refresh_profiles();
        }
    }

    // ---------- éléments d'interface ----------

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        use ff_core::media::{profile_class, profile_accepts, kind_of, MediaKind};
        let l = self.settings.lang;
        let idle = self.items.iter().filter(|i| i.state == State::Idle).count();
        let active = self.items.iter().filter(|i| i.state.is_active()).count();
        let can_run = self.engine.is_some();
        ui.horizontal_wrapped(|ui| {
            if icons::labeled(ui, Icon::Plus, tr(l, "Ajouter des fichiers"), Some(accent()), true, "").clicked() {
                if let Some(files) = rfd::FileDialog::new().add_filter(tr(l, "Médias"), MEDIA_EXT).add_filter(tr(l, "Tous les fichiers"), &["*"]).pick_files() {
                    self.add_files(files);
                }
            }
            ui.add_space(6.0);
            ui.label(tr(l, "Profil :"));
            let current = self.profiles.get(self.selected).map(|p| plabel(l, p)).unwrap_or_default();
            let profiles = self.profiles.clone();
            egui::ComboBox::from_id_salt("profile").width(250.0).selected_text(current).show_ui(ui, |ui| {
                for (class, title) in [(MediaKind::Video, "Vidéo"), (MediaKind::Audio, "Audio"), (MediaKind::Image, "Image")] {
                    ui.label(RichText::new(tr(l, title)).strong().color(accent()));
                    for (i, p) in profiles.iter().enumerate().filter(|(_, p)| profile_class(p) == class) {
                        if ui.selectable_label(i == self.selected, plabel(l, p)).clicked() {
                            self.selected = i;
                            self.settings.last_profile = p.id.clone();
                            self.save_settings();
                            // Le nouveau profil s'applique aux fichiers en attente auxquels il convient.
                            for it in self.items.iter_mut().filter(|it| it.state == State::Idle && it.kind == JobKind::Convert) {
                                if kind_of(&it.input).map(|k| profile_accepts(p, k)).unwrap_or(true) {
                                    it.profile = p.clone();
                                }
                            }
                        }
                    }
                    ui.separator();
                }
            });
            if icons::labeled(ui, Icon::Sliders, tr(l, "Profils…"), None, true, "").clicked() {
                self.show_profiles = true;
            }
            if icons::labeled(ui, Icon::Tools, tr(l, "Outils…"), None, true, "").clicked() {
                self.show_tools = true;
            }
            if icons::labeled(ui, Icon::Film, tr(l, "En ligne…"), None, true, "").clicked() {
                self.show_online = true;
            }
        });
        ui.horizontal_wrapped(|ui| {
            let (r, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
            icons::paint(ui.painter(), r, Icon::Folder, accent());
            ui.label(tr(l, "Sortie :"));
            let txt = self.settings.output_dir.as_ref().map(|d| d.display().to_string()).unwrap_or_else(|| tr(l, "dossier de chaque fichier source").into());
            ui.label(RichText::new(txt).weak());
            if icons::labeled(ui, Icon::Folder, tr(l, "Choisir…"), None, true, "").clicked() {
                if let Some(d) = rfd::FileDialog::new().pick_folder() {
                    self.settings.output_dir = Some(d);
                    self.save_settings();
                }
            }
            if self.settings.output_dir.is_some() && ui.button(tr(l, "Dossier source")).clicked() {
                self.settings.output_dir = None;
                self.save_settings();
            }
        });
        ui.horizontal_wrapped(|ui| {
            if icons::labeled(ui, Icon::Play, &format!("{} ({idle})", tr(l, "Convertir")), Some(green()), idle > 0 && can_run, "").clicked() {
                self.start_all();
            }
            if icons::labeled(ui, Icon::Stop, tr(l, "Tout annuler"), None, active > 0, "").clicked() {
                if let Some((q, _)) = &self.engine {
                    q.cancel_all();
                }
            }
            let resettable = self.items.iter().any(|i| matches!(i.state, State::Failed(_) | State::Cancelled));
            if icons::labeled(ui, Icon::Rotate, tr(l, "Réinitialiser annulés / échecs"), None, resettable, "").clicked() {
                for i in 0..self.items.len() {
                    self.reset_item(i);
                }
            }
            if icons::labeled(ui, Icon::Check, tr(l, "Vider les terminés"), None, self.items.iter().any(|i| i.state.is_final()), "").clicked() {
                self.items.retain(|i| !i.state.is_final());
            }
            if icons::labeled(ui, Icon::Trash, tr(l, "Vider la liste d'attente"), None, idle > 0, "").clicked() {
                self.items.retain(|i| i.state != State::Idle);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let grid = self.settings.icon_view;
                if icons::button(ui, Icon::Grid, 18.0, grid, tr(l, "Affichage en icônes")).clicked() && !grid {
                    self.settings.icon_view = true;
                    self.save_settings();
                }
                if icons::button(ui, Icon::List, 18.0, !grid, tr(l, "Affichage en liste")).clicked() && grid {
                    self.settings.icon_view = false;
                    self.save_settings();
                }
            });
        });
    }

    fn rows(&mut self, ui: &mut egui::Ui) {
        use ff_core::media::{kind_of, profile_accepts, MediaKind};
        let l = self.settings.lang;
        let profiles = self.profiles.clone();
        let can_edit = self.bins.is_some();
        let mut actions: Vec<Action> = Vec::new();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for (idx, it) in self.items.iter_mut().enumerate() {
                let in_kind = kind_of(&it.input).unwrap_or(MediaKind::Video);
                egui::Frame::group(ui.style()).fill(ui.visuals().extreme_bg_color).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        // pastille du type de fichier
                        let (r, _) = ui.allocate_exact_size(egui::vec2(40.0, 40.0), egui::Sense::hover());
                        let col = match in_kind {
                            MediaKind::Video => accent(),
                            MediaKind::Audio => green(),
                            MediaKind::Image => ff_kit::theme::palette(ff_kit::theme::Style::from_id(&self.settings.theme), self.settings.dark).accent.gamma_multiply(0.6),
                        };
                        ui.painter().rect_filled(r, 8.0, col.gamma_multiply(0.18));
                        let ic = if it.kind != JobKind::Convert { Icon::Merge } else { match in_kind { MediaKind::Video => Icon::Film, MediaKind::Audio => Icon::Note, MediaKind::Image => Icon::Picture } };
                        icons::paint(ui.painter(), r.shrink(9.0), ic, col);
                        ui.vertical(|ui| {
                            ui.label(RichText::new(it.display_name()).strong());
                            let tool = it.kind != JobKind::Convert;
                            if it.state == State::Idle && !tool {
                                ui.horizontal_wrapped(|ui| {
                                    egui::ComboBox::from_id_salt(("rowp", it.uid)).width(230.0).selected_text(format!(".{} · {}", it.profile.extension, plabel(l, &it.profile))).show_ui(ui, |ui| {
                                        for p in profiles.iter().filter(|p| profile_accepts(p, in_kind)) {
                                            if ui.selectable_label(it.profile.id == p.id, plabel(l, p)).clicked() {
                                                it.profile = p.clone();
                                            }
                                        }
                                    });
                                    let stem = it.input.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                                    ui.add(egui::TextEdit::singleline(&mut it.out_name).hint_text(stem).desired_width(170.0)).on_hover_text(tr(l, "Nom de sortie (vide = nom du fichier source)"));
                                    if icons::labeled(ui, Icon::Scissors, tr(l, "Retouches…"), None, can_edit, "").clicked() {
                                        actions.push(Action::Edit(idx));
                                    }
                                });
                            } else {
                                let out = it.output.as_ref().and_then(|o| o.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| format!(".{}", it.profile.extension));
                                ui.label(RichText::new(format!("→ {out} · {}", plabel(l, &it.profile))).small().weak());
                            }
                            if let Some(txt) = crate::edit::summary(l, &it.edits) {
                                ui.label(RichText::new(txt).small().color(accent()));
                            }
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            match &it.state {
                                State::Idle | State::Done | State::Failed(_) | State::Cancelled => {
                                    if icons::button(ui, Icon::Close, 14.0, false, tr(l, "Retirer de la liste")).clicked() {
                                        actions.push(Action::Remove(idx));
                                    }
                                    if matches!(it.state, State::Failed(_) | State::Cancelled) && icons::button(ui, Icon::Rotate, 14.0, false, tr(l, "Réinitialiser (remettre en attente)")).clicked() {
                                        actions.push(Action::Reset(idx));
                                    }
                                }
                                State::Queued | State::Running => {
                                    if icons::button(ui, Icon::Stop, 14.0, false, tr(l, "Annuler")).clicked() {
                                        actions.push(Action::Cancel(idx));
                                    }
                                }
                            }
                            if it.state == State::Done && icons::button(ui, Icon::Folder, 16.0, false, tr(l, "Afficher le fichier converti")).clicked() {
                                actions.push(Action::Reveal(idx));
                            }
                            let (label, color) = match &it.state {
                                State::Idle => (tr(l, "En attente").to_string(), None),
                                State::Queued => (tr(l, "En file").to_string(), None),
                                State::Running => (format!("{:.0} %  ·  x{:.1}  ·  {}", it.progress.percent, it.progress.speed, fmt_secs(it.progress.out_time_secs)), Some(accent())),
                                State::Done => (tr(l, "Terminé").to_string(), Some(green())),
                                State::Failed(_) => (tr(l, "Échec").to_string(), Some(red())),
                                State::Cancelled => (tr(l, "Annulé").to_string(), None),
                            };
                            let mut rt = RichText::new(label);
                            if let Some(c) = color {
                                rt = rt.color(c).strong();
                            }
                            let resp = ui.label(rt);
                            if let State::Failed(msg) = &it.state {
                                resp.on_hover_text(msg);
                            }
                            if it.state.is_active() || it.state == State::Done {
                                ui.add(egui::ProgressBar::new((it.progress.percent / 100.0).clamp(0.0, 1.0)).desired_width(160.0).fill(if it.state == State::Done { green() } else { accent() }));
                            }
                        });
                    });
                });
            }
        });
        self.do_actions(actions);
    }

    fn do_actions(&mut self, actions: Vec<Action>) {
        for a in actions.into_iter().rev() {
            match a {
                Action::Remove(i) => {
                    self.items.remove(i);
                }
                Action::Cancel(i) => {
                    if let (Some((q, _)), Some(id)) = (&self.engine, self.items[i].engine_id) {
                        q.cancel(id);
                    }
                }
                Action::Reset(i) => self.reset_item(i),
                Action::Reveal(i) => {
                    if let Some(o) = &self.items[i].output {
                        reveal(o);
                    }
                }
                Action::Edit(i) => self.open_edit(i),
            }
        }
    }

    /// Affichage en grandes icônes : une carte par fichier.
    fn cards(&mut self, ui: &mut egui::Ui) {
        use ff_core::media::{kind_of, profile_accepts, MediaKind};
        let l = self.settings.lang;
        let profiles = self.profiles.clone();
        let can_edit = self.bins.is_some();
        let mut actions: Vec<Action> = Vec::new();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(10.0, 10.0);
                for (idx, it) in self.items.iter_mut().enumerate() {
                    let in_kind = kind_of(&it.input).unwrap_or(MediaKind::Video);
                    let col = match in_kind {
                        MediaKind::Video => accent(),
                        MediaKind::Audio => green(),
                        MediaKind::Image => accent().gamma_multiply(0.6),
                    };
                    egui::Frame::group(ui.style()).fill(ui.visuals().extreme_bg_color).show(ui, |ui| {
                        ui.set_width(190.0);
                        ui.set_min_height(170.0);
                        ui.vertical_centered(|ui| {
                            let (r, _) = ui.allocate_exact_size(egui::vec2(72.0, 72.0), egui::Sense::hover());
                            ui.painter().rect_filled(r, 14.0, col.gamma_multiply(0.18));
                            let ic = if it.kind != JobKind::Convert { Icon::Merge } else { match in_kind { MediaKind::Video => Icon::Film, MediaKind::Audio => Icon::Note, MediaKind::Image => Icon::Picture } };
                            icons::paint(ui.painter(), r.shrink(16.0), ic, col);
                            let name = it.display_name();
                            let short: String = if name.chars().count() > 24 { format!("{}…", name.chars().take(23).collect::<String>()) } else { name.clone() };
                            ui.label(RichText::new(short).strong()).on_hover_text(name);
                            if it.state == State::Idle && it.kind == JobKind::Convert {
                                egui::ComboBox::from_id_salt(("cardp", it.uid)).width(160.0).selected_text(format!(".{} · {}", it.profile.extension, plabel(l, &it.profile))).show_ui(ui, |ui| {
                                    for p in profiles.iter().filter(|p| profile_accepts(p, in_kind)) {
                                        if ui.selectable_label(it.profile.id == p.id, plabel(l, p)).clicked() {
                                            it.profile = p.clone();
                                        }
                                    }
                                });
                            } else {
                                ui.label(RichText::new(format!(".{} · {}", it.profile.extension, plabel(l, &it.profile))).small().weak());
                            }
                            let (label, color) = match &it.state {
                                State::Idle => (tr(l, "En attente").to_string(), None),
                                State::Queued => (tr(l, "En file").to_string(), None),
                                State::Running => (format!("{:.0} %", it.progress.percent), Some(accent())),
                                State::Done => (tr(l, "Terminé").to_string(), Some(green())),
                                State::Failed(_) => (tr(l, "Échec").to_string(), Some(red())),
                                State::Cancelled => (tr(l, "Annulé").to_string(), None),
                            };
                            let mut rt = RichText::new(label);
                            if let Some(c) = color {
                                rt = rt.color(c).strong();
                            }
                            let resp = ui.label(rt);
                            if let State::Failed(msg) = &it.state {
                                resp.on_hover_text(msg);
                            }
                            if it.state.is_active() || it.state == State::Done {
                                ui.add(egui::ProgressBar::new((it.progress.percent / 100.0).clamp(0.0, 1.0)).desired_width(150.0).fill(if it.state == State::Done { green() } else { accent() }));
                            }
                            ui.horizontal(|ui| {
                                if it.state == State::Idle && it.kind == JobKind::Convert && icons::button(ui, Icon::Scissors, 14.0, false, tr(l, "Retouches…")).clicked() && can_edit {
                                    actions.push(Action::Edit(idx));
                                }
                                if it.state == State::Done && icons::button(ui, Icon::Folder, 14.0, false, tr(l, "Afficher le fichier converti")).clicked() {
                                    actions.push(Action::Reveal(idx));
                                }
                                if matches!(it.state, State::Failed(_) | State::Cancelled) && icons::button(ui, Icon::Rotate, 14.0, false, tr(l, "Réinitialiser (remettre en attente)")).clicked() {
                                    actions.push(Action::Reset(idx));
                                }
                                if it.state.is_active() {
                                    if icons::button(ui, Icon::Stop, 14.0, false, tr(l, "Annuler")).clicked() {
                                        actions.push(Action::Cancel(idx));
                                    }
                                } else if icons::button(ui, Icon::Close, 14.0, false, tr(l, "Retirer de la liste")).clicked() {
                                    actions.push(Action::Remove(idx));
                                }
                            });
                        });
                    });
                }
            });
        });
        self.do_actions(actions);
    }
}

const TEMPLATES: &[(&str, &str, &str)] = &[
    ("Vidéo H.264 (qualité CRF 23)", "mp4", "-c:v libx264 -preset medium -crf 23 -c:a aac -b:a 128k -movflags +faststart"),
    ("Vidéo H.265 (qualité CRF 28)", "mp4", "-c:v libx265 -preset medium -crf 28 -tag:v hvc1 -c:a aac -b:a 128k"),
    ("Vidéo VP9 + Opus", "webm", "-c:v libvpx-vp9 -crf 32 -b:v 0 -c:a libopus -b:a 96k"),
    ("Redimensionner en 720p", "mp4", "-c:v libx264 -crf 23 -vf scale=-2:720 -c:a aac -b:a 128k"),
    ("GIF animé (480 px, 12 i/s)", "gif", "-vf fps=12,scale=480:-1:flags=lanczos -an"),
    ("Audio MP3 (VBR haute qualité)", "mp3", "-vn -c:a libmp3lame -q:a 2"),
    ("Extraire la piste audio sans réencodage", "mka", "-vn -c:a copy"),
    ("Copie des flux (changement de conteneur)", "mkv", "-c copy"),
];

impl eframe::App for App {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Suit un changement de thème fait depuis le lecteur (fichier de réglages partagé, relu toutes les 2 s).
        thread_local!(static CHK: std::cell::Cell<Option<std::time::Instant>> = const { std::cell::Cell::new(None) });
        if CHK.with(|c| c.get()).map_or(true, |t| t.elapsed().as_secs() >= 2) {
            CHK.with(|c| c.set(Some(std::time::Instant::now())));
            let sh = ff_profiles::load();
            if sh.theme != self.settings.theme || sh.dark != self.settings.dark {
                self.settings.theme = sh.theme;
                self.settings.dark = sh.dark;
                self.apply_theme();
            }
            root.ctx().request_repaint_after(std::time::Duration::from_secs(2));
        }
        let ctx = root.ctx().clone();
        let ctx = &ctx;
        let l = self.settings.lang;
        self.poll_events();
        if ctx.input(|i| i.key_pressed(egui::Key::F1)) {
            ff_kit::help::trigger(ctx, "mediaforge", "premiers-pas");
        }
        ff_kit::help::show_error(ctx);
        self.poll_full();
        if self.full_dl.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }

        // Fichiers transmis par une autre instance (menu contextuel, « Ouvrir avec »)
        let mut forwarded: Vec<PathBuf> = Vec::new();
        if let Some(rx) = &self.inbox {
            while let Ok(b) = rx.try_recv() {
                forwarded.extend(b);
            }
        }
        if !forwarded.is_empty() {
            self.add_files(forwarded.into_iter().flat_map(expand_dropped).filter(|p| p.is_file()).collect());
        }

        // Glisser-déposer
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        if !dropped.is_empty() {
            let files: Vec<PathBuf> = dropped.into_iter().flat_map(expand_dropped).filter(|p| p.is_file()).collect();
            self.add_files(files);
        }
        let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());

        let pal = crate::look::pal();
        egui::Panel::top("top").show(root, |ui| {
            let band = pal.header;
            let fg = band.map(ff_kit::theme::on_color).unwrap_or(pal.text);
            egui::Frame::new().fill(band.unwrap_or(egui::Color32::TRANSPARENT)).inner_margin(egui::Margin::symmetric(10, 6)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(egui::vec2(26.0, 26.0), egui::Sense::hover());
                    icons::paint(ui.painter(), r, Icon::Merge, if band.is_some() { fg } else { accent() });
                    ui.heading(RichText::new("MediaForge").color(if band.is_some() { fg } else { accent() }).strong());
                    ui.label(RichText::new(tr(l, "convertisseur de médias")).color(fg.gamma_multiply(0.75)));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if icons::labeled(ui, Icon::Gear, tr(l, "Réglages"), None, true, "").clicked() {
                            self.show_settings = true;
                        }
                        ff_kit::help::button(ui, "mediaforge", "premiers-pas", tr(l, "Aide (F1)"));
                        let (ic, tip) = if self.settings.dark { (Icon::Sun, tr(l, "Thème clair")) } else { (Icon::Moon, tr(l, "Thème sombre")) };
                        if icons::labeled(ui, ic, "", None, true, tip).clicked() {
                            self.settings.dark = !self.settings.dark;
                            self.apply_theme();
                            self.save_settings();
                        }
                    });
                });
            });
            ui.add_space(4.0);
            self.toolbar(ui);
            ui.add_space(4.0);
        });

        egui::Panel::bottom("bottom").show(root, |ui| {
            ui.horizontal(|ui| {
                let n = |f: fn(&State) -> bool| self.items.iter().filter(|i| f(&i.state)).count();
                ui.label(trf(
                    l,
                    "{total} fichier(s) · {run} en cours · {done} terminé(s) · {fail} échec(s)",
                    &[
                        ("{total}", self.items.len().to_string()),
                        ("{run}", n(|s| matches!(s, State::Running)).to_string()),
                        ("{done}", n(|s| matches!(s, State::Done)).to_string()),
                        ("{fail}", n(|s| matches!(s, State::Failed(_))).to_string()),
                    ],
                ));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(format!("v{VERSION}")).small().color(Color32::GRAY));
                    if icons::labeled(ui, Icon::Doc, tr(l, "Journal"), None, true, "").clicked() {
                        self.show_log = true;
                    }
                });
            });
        });

        egui::CentralPanel::default().show(root, |ui| {
            if let Some(err) = self.bins_err.clone() {
                ui.horizontal_wrapped(|ui| {
                    ui.colored_label(red(), format!("{}.", tr(l, &err)));
                    if ui.button(tr(l, "Choisir le dossier FFmpeg…")).clicked() {
                        if let Some(d) = rfd::FileDialog::new().pick_folder() {
                            self.settings.ffmpeg_dir = Some(d);
                            self.save_settings();
                            self.init_engine();
                        }
                    }
                    if ui.button(tr(l, "Réessayer")).clicked() {
                        self.init_engine();
                    }
                });
                ui.separator();
            }
            if self.items.is_empty() {
                ui.centered_and_justified(|ui| {
                    ui.label(RichText::new(tr(l, "Glissez des fichiers ou un dossier ici,\nou cliquez sur « Ajouter des fichiers ».")).size(18.0).weak());
                });
            } else if self.settings.icon_view {
                self.cards(ui);
            } else {
                self.rows(ui);
            }
            if hovering {
                let r = ui.max_rect();
                ui.painter().rect_filled(r, 8.0, Color32::from_rgba_unmultiplied(0xE8, 0x59, 0x0C, 60));
                ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, tr(l, "Déposez pour ajouter"), egui::FontId::proportional(24.0), Color32::WHITE);
            }
        });

        if self.show_settings {
            self.settings_window(ctx);
        }
        if self.full_prompt {
            self.full_prompt_window(ctx);
        }
        if self.show_profiles {
            self.profiles_window(ctx);
        }
        if self.show_log {
            self.log_window(ctx);
        }
        if self.show_tools {
            self.tools_window(ctx);
        }
        if self.show_online {
            self.online_window(ctx);
        }
        if self.edit.is_some() {
            self.edit_window(ctx);
        }
    }
}

/// Décode un PPM binaire (P6) : (largeur, hauteur, octets RGB).
fn parse_ppm(b: &[u8]) -> Result<(usize, usize, Vec<u8>), String> {
    let mut pos = 0;
    let mut tok = || -> Option<String> {
        while pos < b.len() && b[pos].is_ascii_whitespace() {
            pos += 1;
        }
        let s = pos;
        while pos < b.len() && !b[pos].is_ascii_whitespace() {
            pos += 1;
        }
        (pos > s).then(|| String::from_utf8_lossy(&b[s..pos]).into_owned())
    };
    let bad = || "aperçu illisible".to_string();
    if tok().as_deref() != Some("P6") {
        return Err(bad());
    }
    let w: usize = tok().and_then(|t| t.parse().ok()).ok_or_else(bad)?;
    let h: usize = tok().and_then(|t| t.parse().ok()).ok_or_else(bad)?;
    let _max = tok();
    let start = pos + 1;
    let data = b.get(start..start + w * h * 3).ok_or_else(bad)?;
    Ok((w, h, data.to_vec()))
}
