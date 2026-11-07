//! Vidéos en ligne : listes (YouTube non répertoriées, playlists…), lecture dans le lecteur, téléchargement, conversion. v0.9.0
use crate::app::App;
use crate::i18n::{tr, trf};
use crate::look::{accent, green, red};
use eframe::egui::{self, Color32, RichText};
use ff_online::{Entry, Listing, Quality};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub struct Shared {
    fetching: bool,
    fetched: Option<Result<Listing, String>>,
    installing: bool,
    installed: Option<Result<String, String>>,
    dl_running: bool,
    dl_total: usize,
    dl_done: usize,
    dl_name: String,
    dl_pct: f32,
    dl_errors: Vec<String>,
    dl_files: Vec<PathBuf>,
    cancel: bool,
}

pub struct OnlineState {
    url: String,
    current: Option<Listing>,
    sel: Vec<bool>,
    saved: Vec<Listing>,
    name: String,
    quality: usize,
    to_queue: bool,
    ytdlp: Option<String>,
    msg: String,
    sh: Arc<Mutex<Shared>>,
    svc: Arc<Mutex<Option<Result<Vec<String>, String>>>>,
    svc_loading: bool,
    svc_filter: String,
}

impl Default for OnlineState {
    fn default() -> Self {
        OnlineState {
            url: String::new(),
            current: None,
            sel: Vec::new(),
            saved: ff_online::load_lists(),
            name: String::new(),
            quality: 0,
            to_queue: false,
            ytdlp: ff_online::version(),
            msg: String::new(),
            sh: Arc::new(Mutex::new(Shared::default())),
            svc: Arc::new(Mutex::new(None)),
            svc_loading: false,
            svc_filter: String::new(),
        }
    }
}

fn fmt_dur(d: Option<f64>) -> String {
    match d {
        Some(s) if s >= 3600.0 => format!("{}:{:02}:{:02}", s as u64 / 3600, s as u64 % 3600 / 60, s as u64 % 60),
        Some(s) => format!("{}:{:02}", s as u64 / 60, s as u64 % 60),
        None => String::new(),
    }
}

fn player_exe() -> PathBuf {
    let n = format!("mediaforge-player{}", std::env::consts::EXE_SUFFIX);
    std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join(&n))).filter(|p| p.exists()).unwrap_or_else(|| PathBuf::from(n))
}

impl OnlineState {
    fn set_current(&mut self, l: Listing) {
        self.sel = vec![true; l.entries.len()];
        self.name = l.name.clone();
        self.url = l.url.clone();
        self.current = Some(l);
    }

    fn start_fetch(&mut self, url: String) {
        let sh = self.sh.clone();
        if let Ok(mut s) = sh.lock() {
            s.fetching = true;
            s.fetched = None;
        }
        std::thread::spawn(move || {
            let r = ff_online::fetch(&url);
            if let Ok(mut s) = sh.lock() {
                s.fetching = false;
                s.fetched = Some(r);
            }
        });
    }

    fn start_install(&mut self) {
        let sh = self.sh.clone();
        if let Ok(mut s) = sh.lock() {
            s.installing = true;
        }
        std::thread::spawn(move || {
            let r = ff_online::install_latest();
            if let Ok(mut s) = sh.lock() {
                s.installing = false;
                s.installed = Some(r);
            }
        });
    }

    fn selected(&self) -> Vec<Entry> {
        self.current.as_ref().map(|l| l.entries.iter().zip(&self.sel).filter(|(_, s)| **s).map(|(e, _)| e.clone()).collect()).unwrap_or_default()
    }

    fn start_download(&mut self, out: PathBuf, ffmpeg_dir: Option<PathBuf>) {
        let list = self.selected();
        if list.is_empty() {
            return;
        }
        let q = [Quality::Video1080, Quality::VideoBest, Quality::AudioMp3][self.quality.min(2)];
        let sh = self.sh.clone();
        if let Ok(mut s) = sh.lock() {
            *s = Shared { dl_running: true, dl_total: list.len(), fetched: s.fetched.take(), installed: s.installed.take(), ..Default::default() };
        }
        std::thread::spawn(move || {
            for e in list {
                if sh.lock().map(|s| s.cancel).unwrap_or(true) {
                    break;
                }
                if let Ok(mut s) = sh.lock() {
                    s.dl_name = e.title.clone();
                    s.dl_pct = 0.0;
                }
                let sh2 = sh.clone();
                let r = ff_online::download(&e.url, &out, ffmpeg_dir.as_deref(), q, &mut |p| {
                    if let Ok(mut s) = sh2.lock() {
                        s.dl_pct = p;
                    }
                });
                if let Ok(mut s) = sh.lock() {
                    s.dl_done += 1;
                    match r {
                        Ok(f) => s.dl_files.push(f),
                        Err(m) => s.dl_errors.push(format!("{} : {m}", e.title)),
                    }
                }
            }
            if let Ok(mut s) = sh.lock() {
                s.dl_running = false;
            }
        });
    }
}

impl App {
    pub(crate) fn online_window(&mut self, ctx: &egui::Context) {
        let l = self.settings.lang;
        let mut open = self.show_online;
        let mut to_queue_files: Vec<PathBuf> = Vec::new();

        // Résultats des tâches de fond.
        {
            let st = &mut self.online;
            let (fetched, installed, finished_files) = match st.sh.lock() {
                Ok(mut s) => (s.fetched.take(), s.installed.take(), if !s.dl_running && !s.dl_files.is_empty() && s.dl_done >= s.dl_total { Some((std::mem::take(&mut s.dl_files), s.dl_errors.len())) } else { None }),
                Err(_) => (None, None, None),
            };
            match fetched {
                Some(Ok(lst)) => {
                    st.msg = trf(l, "{n} vidéo(s) trouvée(s).", &[("{n}", lst.entries.len().to_string())]);
                    st.set_current(lst);
                }
                Some(Err(e)) => st.msg = e,
                None => {}
            }
            match installed {
                Some(Ok(v)) => {
                    st.ytdlp = Some(v.clone());
                    st.msg = format!("yt-dlp {v}");
                }
                Some(Err(e)) => st.msg = e,
                None => {}
            }
            if let Some((files, errs)) = finished_files {
                st.msg = format!("{} {}", trf(l, "{n} fichier(s) téléchargé(s).", &[("{n}", files.len().to_string())]), if errs > 0 { trf(l, "({n} erreur(s))", &[("{n}", errs.to_string())]) } else { String::new() });
                if st.to_queue {
                    to_queue_files = files;
                }
            }
        }

        let out_dir = self.settings.online_dir.clone().or_else(dirs::download_dir).unwrap_or_else(|| ff_profiles::data_dir().join("downloads"));
        let ffmpeg_dir = self.bins.as_ref().and_then(|b| b.ffmpeg.parent().map(|p| p.to_path_buf()));
        let mut want_fetch: Option<String> = None;
        let mut new_dir: Option<std::path::PathBuf> = None;
        let mut reset_dir = false;

        egui::Window::new(tr(l, "Vidéos en ligne")).open(&mut open).default_size([680.0, 620.0]).scroll([false, true]).show(ctx, |ui| {
            ff_kit::help::links(ui, "mediaforge", &[("Télécharger", "en-ligne"), ("Listes enregistrées", "en-ligne-listes")]);
            let st = &mut self.online;
            let (fetching, installing, running, total, done, pct, name, errors) = match st.sh.lock() {
                Ok(s) => (s.fetching, s.installing, s.dl_running, s.dl_total, s.dl_done, s.dl_pct, s.dl_name.clone(), s.dl_errors.clone()),
                Err(_) => (false, false, false, 0, 0, 0.0, String::new(), vec![]),
            };
            if fetching || installing || running {
                ctx.request_repaint_after(std::time::Duration::from_millis(150));
            }
            // yt-dlp
            ui.horizontal(|ui| {
                match &st.ytdlp {
                    Some(v) => ui.label(RichText::new(format!("yt-dlp {v}")).small().weak()),
                    None => ui.colored_label(red(), tr(l, "yt-dlp est introuvable.")),
                };
                let lbl = if st.ytdlp.is_some() { tr(l, "Mettre à jour yt-dlp") } else { tr(l, "Installer yt-dlp") };
                if ui.add_enabled(!installing, egui::Button::new(lbl)).clicked() {
                    st.start_install();
                }
                if installing {
                    ui.spinner();
                }
            });
            ui.label(RichText::new(tr(l, "À utiliser pour vos propres vidéos ou celles dont vous avez le droit : le téléchargement n'est pas autorisé par YouTube pour les vidéos d'autrui.")).small().weak());
            egui::CollapsingHeader::new(tr(l, "Services pris en charge")).show(ui, |ui| {
                for (k, v) in ff_online::SERVICES {
                    ui.label(RichText::new(*k).strong());
                    ui.label(RichText::new(*v).small());
                }
                ui.add_space(4.0);
                let loaded = st.svc.lock().map(|g| g.is_some()).unwrap_or(false);
                if !loaded && ui.add_enabled(!st.svc_loading && st.ytdlp.is_some(), egui::Button::new(tr(l, "Liste complète (yt-dlp)"))).clicked() {
                    st.svc_loading = true;
                    let slot = st.svc.clone();
                    std::thread::spawn(move || {
                        let r = ff_online::extractors();
                        if let Ok(mut g) = slot.lock() {
                            *g = Some(r);
                        }
                    });
                }
                if st.svc_loading && !loaded {
                    ui.spinner();
                    ctx.request_repaint_after(std::time::Duration::from_millis(200));
                }
                if let Ok(g) = st.svc.lock() {
                    match &*g {
                        Some(Ok(v)) => {
                            ui.horizontal(|ui| {
                                ui.label(format!("{} {}", v.len(), tr(l, "sites")));
                                ui.add(egui::TextEdit::singleline(&mut st.svc_filter).hint_text(tr(l, "Filtrer…")).desired_width(180.0));
                            });
                            let f = st.svc_filter.to_lowercase();
                            let list: Vec<&String> = v.iter().filter(|n| f.is_empty() || n.to_lowercase().contains(&f)).collect();
                            egui::ScrollArea::vertical().id_salt("svclist").max_height(160.0).show_rows(ui, 16.0, list.len(), |ui, r| {
                                for n in &list[r] {
                                    ui.label(RichText::new(n.as_str()).small());
                                }
                            });
                        }
                        Some(Err(e)) => {
                            ui.colored_label(red(), e);
                        }
                        None => {}
                    }
                }
            });
            ui.separator();

            // Listes enregistrées
            ui.horizontal(|ui| {
                ui.label(tr(l, "Listes enregistrées :"));
                let mut pick: Option<usize> = None;
                egui::ComboBox::from_id_salt("savedlists").width(260.0).selected_text(if st.saved.is_empty() { tr(l, "aucune").to_string() } else { tr(l, "Choisir…").to_string() }).show_ui(ui, |ui| {
                    for (i, s) in st.saved.iter().enumerate() {
                        if ui.selectable_label(false, format!("{} ({})", s.name, s.entries.len())).clicked() {
                            pick = Some(i);
                        }
                    }
                });
                if let Some(i) = pick {
                    let s = st.saved[i].clone();
                    st.set_current(s);
                }
            });

            // URL
            ui.horizontal(|ui| {
                ui.label("URL :");
                let r = ui.add(egui::TextEdit::singleline(&mut st.url).desired_width(380.0).hint_text("https://www.youtube.com/playlist?list=…"));
                let go = ui.add_enabled(!fetching && ff_online::is_url(st.url.trim()), egui::Button::new(RichText::new(tr(l, "Analyser")).color(Color32::WHITE)).fill(accent())).clicked() || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && ff_online::is_url(st.url.trim()));
                if go && !fetching {
                    want_fetch = Some(st.url.trim().to_string());
                }
                if fetching {
                    ui.spinner();
                }
            });

            // Contenu de la liste
            if let Some(cur) = st.current.clone() {
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.small_button(tr(l, "Tout")).clicked() {
                        st.sel.iter_mut().for_each(|s| *s = true);
                    }
                    if ui.small_button(tr(l, "Aucun")).clicked() {
                        st.sel.iter_mut().for_each(|s| *s = false);
                    }
                    ui.label(RichText::new(trf(l, "{n} sélectionnée(s) sur {t}", &[("{n}", st.sel.iter().filter(|s| **s).count().to_string()), ("{t}", cur.entries.len().to_string())])).weak());
                });
                egui::ScrollArea::vertical().id_salt("onl").max_height(220.0).show(ui, |ui| {
                    for (i, e) in cur.entries.iter().enumerate() {
                        ui.horizontal(|ui| {
                            if let Some(s) = st.sel.get_mut(i) {
                                ui.checkbox(s, "");
                            }
                            ui.label(&e.title);
                            ui.label(RichText::new(fmt_dur(e.duration)).small().weak());
                        });
                    }
                });
                // Enregistrer / exporter
                ui.horizontal(|ui| {
                    ui.label(tr(l, "Nom de la liste :"));
                    ui.add(egui::TextEdit::singleline(&mut st.name).desired_width(200.0));
                    if ui.button(tr(l, "Enregistrer la liste")).clicked() && !st.name.trim().is_empty() {
                        let mut lst = cur.clone();
                        lst.name = st.name.trim().to_string();
                        match st.saved.iter_mut().find(|s| s.name == lst.name) {
                            Some(s) => *s = lst,
                            None => st.saved.push(lst),
                        }
                        st.msg = match ff_online::save_lists(&st.saved) {
                            Ok(_) => tr(l, "Liste enregistrée.").to_string(),
                            Err(e) => e,
                        };
                    }
                    if ui.button(tr(l, "Resynchroniser")).clicked() && !fetching {
                        want_fetch = Some(cur.url.clone());
                    }
                    if ui.button(tr(l, "Supprimer la liste")).clicked() {
                        st.saved.retain(|s| s.name != st.name.trim());
                        let _ = ff_online::save_lists(&st.saved);
                    }
                    if ui.button(tr(l, "Exporter .m3u")).clicked() {
                        if let Some(p) = rfd::FileDialog::new().set_file_name(format!("{}.m3u", st.name.trim())).save_file() {
                            st.msg = match std::fs::write(&p, ff_online::to_m3u(&cur)) {
                                Ok(_) => tr(l, "Liste exportée.").to_string(),
                                Err(e) => e.to_string(),
                            };
                        }
                    }
                });
                ui.separator();
                // Lecture / téléchargement
                let sel = st.selected();
                ui.horizontal(|ui| {
                    if ui.add_enabled(!sel.is_empty(), egui::Button::new(tr(l, "Lire dans le lecteur"))).clicked() {
                        let urls: Vec<String> = sel.iter().map(|e| e.url.clone()).collect();
                        st.msg = match std::process::Command::new(player_exe()).args(urls).spawn() {
                            Ok(_) => String::new(),
                            Err(e) => format!("mediaforge-player : {e}"),
                        };
                    }
                    ui.label(tr(l, "Qualité :"));
                    egui::ComboBox::from_id_salt("onlq").selected_text(tr(l, ["Vidéo MP4 (≤ 1080p)", "Vidéo meilleure qualité (MKV)", "Audio MP3"][st.quality.min(2)])).show_ui(ui, |ui| {
                        for (i, n) in ["Vidéo MP4 (≤ 1080p)", "Vidéo meilleure qualité (MKV)", "Audio MP3"].iter().enumerate() {
                            ui.selectable_value(&mut st.quality, i, tr(l, n));
                        }
                    });
                });
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{} {}", tr(l, "Dossier :"), out_dir.display())).small().weak());
                    if ui.button(tr(l, "Choisir…")).clicked() {
                        new_dir = rfd::FileDialog::new().set_directory(&out_dir).pick_folder();
                    }
                    if ui.button(tr(l, "Par défaut")).clicked() {
                        reset_dir = true;
                    }
                    ui.checkbox(&mut st.to_queue, tr(l, "Ajouter à la file de conversion"));
                });
                if running {
                    ui.add(egui::ProgressBar::new(if total > 0 { (done as f32 + pct / 100.0) / total as f32 } else { 0.0 }).text(format!("{}/{} · {}", done + 1, total, name)));
                    if ui.button(tr(l, "Annuler")).clicked() {
                        if let Ok(mut s) = st.sh.lock() {
                            s.cancel = true;
                        }
                    }
                } else if ui.add_enabled(!sel.is_empty() && st.ytdlp.is_some(), egui::Button::new(RichText::new(tr(l, "Télécharger la sélection")).color(Color32::WHITE)).fill(green())).clicked() {
                    st.msg.clear();
                    st.start_download(out_dir.clone(), ffmpeg_dir.clone());
                }
                for e in errors.iter().take(5) {
                    ui.colored_label(red(), e);
                }
            }
            if !st.msg.is_empty() {
                ui.label(RichText::new(&st.msg).weak());
            }
        });
        self.show_online = open;
        if new_dir.is_some() || reset_dir {
            self.settings.online_dir = new_dir;
            self.save_settings();
        }
        if let Some(u) = want_fetch {
            self.online.msg.clear();
            self.online.start_fetch(u);
        }
        if !to_queue_files.is_empty() {
            self.add_files(to_queue_files);
        }
    }
}
