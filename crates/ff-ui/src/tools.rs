//! Outils : fusion, ajout de piste audio, mixage audio, renommage par lots. v0.3.0
use crate::app::{plabel, App, Item, State};
#[allow(unused_imports)]
use crate::look::{accent, green, red};
use crate::i18n::{tr, trf};
use crate::util::{reveal, MEDIA_EXT};
use eframe::egui::{self, Color32, RichText};
use ff_core::rename::{apply, plan, CaseMode, RenameRule};
use ff_core::{builtin_by_id, unique_output, Edits, JobKind, Lang, Profile, Progress};
use std::path::{Path, PathBuf};


#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Concat,
    Mux,
    Mix,
    Rename,
    Images,
}

pub struct ToolsState {
    pub tab: Tab,
    concat_files: Vec<PathBuf>,
    concat_copy: bool,
    concat_name: String,
    mux_video: Option<PathBuf>,
    mux_audio: Option<PathBuf>,
    mux_name: String,
    mix_files: Vec<PathBuf>,
    mix_profile: String,
    mix_name: String,
    ren_files: Vec<PathBuf>,
    rule: RenameRule,
    ren_confirm: bool,
    pub img: crate::batch::ImgState,
    msg: String,
}

impl Default for ToolsState {
    fn default() -> Self {
        Self {
            tab: Tab::Concat,
            concat_files: Vec::new(),
            concat_copy: false,
            concat_name: "fusion".into(),
            mux_video: None,
            mux_audio: None,
            mux_name: "video_audio".into(),
            mix_files: Vec::new(),
            mix_profile: "mp3".into(),
            mix_name: "mixage".into(),
            ren_files: Vec::new(),
            rule: RenameRule { start: 1, digits: 3, number_at_end: true, no_sep: true, ..Default::default() },
            ren_confirm: false,
            img: Default::default(),
            msg: String::new(),
        }
    }
}

fn ext_of(p: &Path) -> String {
    p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_else(|| "mkv".into())
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Liste ordonnée de fichiers : ajout, montée/descente, retrait.
fn file_list(ui: &mut egui::Ui, l: Lang, id: &str, files: &mut Vec<PathBuf>, filter: &[&str]) {
    if ui.button(tr(l, "+ Ajouter des fichiers")).clicked() {
        let mut dlg = rfd::FileDialog::new();
        if !filter.is_empty() {
            dlg = dlg.add_filter(tr(l, "Médias"), filter);
        }
        if let Some(f) = dlg.pick_files() {
            files.extend(f);
        }
    }
    let (mut up, mut down, mut del) = (None, None, None);
    egui::ScrollArea::vertical().id_salt(id).max_height(170.0).show(ui, |ui| {
        for (i, f) in files.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(format!("{}.", i + 1));
                if ui.small_button("▲").clicked() && i > 0 {
                    up = Some(i);
                }
                if ui.small_button("▼").clicked() && i + 1 < files.len() {
                    down = Some(i);
                }
                if ui.small_button("✕").clicked() {
                    del = Some(i);
                }
                ui.label(file_name(f));
            });
        }
    });
    if let Some(i) = up {
        files.swap(i, i - 1);
    }
    if let Some(i) = down {
        files.swap(i, i + 1);
    }
    if let Some(i) = del {
        files.remove(i);
    }
}

fn pick_one(ui: &mut egui::Ui, l: Lang, label: &str, slot: &mut Option<PathBuf>, exts: &[&str]) {
    ui.horizontal(|ui| {
        ui.label(label);
        if ui.button(tr(l, "Choisir…")).clicked() {
            if let Some(f) = rfd::FileDialog::new().add_filter(tr(l, "Médias"), exts).pick_file() {
                *slot = Some(f);
            }
        }
        ui.label(RichText::new(slot.as_ref().map(|p| file_name(p)).unwrap_or_else(|| tr(l, "aucun").into())).weak());
    });
}

impl App {
    /// Crée un élément « outil » en attente : sortie imposée, unique, dans le dossier de sortie ou celui de la source.
    fn add_tool_item(&mut self, input: PathBuf, title: String, kind: JobKind, profile: Profile, name: &str) {
        let dir = self.settings.output_dir.clone().or_else(|| input.parent().map(Path::to_path_buf)).unwrap_or_default();
        let stem = if name.trim().is_empty() { "sortie" } else { name.trim() };
        let mut out = unique_output(dir.join(format!("{stem}.{}", profile.extension)), &input);
        let mut n = 1;
        while self.items.iter().any(|i| !i.state.is_final() && i.output.as_ref() == Some(&out)) && n < 10_000 {
            out = unique_output(dir.join(format!("{stem} ({n}).{}", profile.extension)), &input);
            n += 1;
        }
        self.items.push(Item {
            uid: self.next_uid,
            engine_id: None,
            input,
            profile,
            output: Some(out),
            state: State::Idle,
            progress: Progress::default(),
            kind,
            edits: Edits::default(),
            title: Some(title),
            fixed_output: true,
            out_name: String::new(),
        });
        self.next_uid += 1;
    }

    pub(crate) fn tools_window(&mut self, ctx: &egui::Context) {
        let l = self.settings.lang;
        let mut open = self.show_tools;
        let mut submit: Option<Tab> = None;
        egui::Window::new(tr(l, "Outils")).open(&mut open).default_size([620.0, 600.0]).scroll([false, true]).show(ctx, |ui| {
            ff_kit::help::links(ui, "mediaforge", &[("Fusion", "fusion"), ("Piste audio", "piste-audio"), ("Mixage", "mixage"), ("Renommage", "renommer-fichiers"), ("Images par lots", "images-par-lots")]);
            ui.horizontal(|ui| {
                for (t, label) in [(Tab::Concat, "Fusion"), (Tab::Mux, "Ajouter une piste audio"), (Tab::Mix, "Mixer des audios"), (Tab::Rename, "Renommer par lots"), (Tab::Images, "Images par lots")] {
                    if ui.selectable_label(self.tools.tab == t, tr(l, label)).clicked() {
                        self.tools.tab = t;
                        self.tools.msg.clear();
                    }
                }
            });
            ui.separator();
            let cur = self.profiles.get(self.selected).cloned();
            match self.tools.tab {
                Tab::Concat => {
                    ui.label(tr(l, "Fusionne des vidéos ou des audios bout à bout, dans l'ordre de la liste."));
                    file_list(ui, l, "cf", &mut self.tools.concat_files, MEDIA_EXT);
                    ui.radio_value(&mut self.tools.concat_copy, false, format!("{} ({})", tr(l, "Réencoder avec le profil choisi"), cur.as_ref().map(|p| plabel(l, p)).unwrap_or_default()));
                    ui.radio_value(&mut self.tools.concat_copy, true, tr(l, "Sans réencodage (mêmes codecs et paramètres requis)"));
                    ui.horizontal(|ui| {
                        ui.label(tr(l, "Nom de sortie :"));
                        ui.add(egui::TextEdit::singleline(&mut self.tools.concat_name).desired_width(200.0));
                    });
                    if ui.add_enabled(self.tools.concat_files.len() >= 2, egui::Button::new(RichText::new(tr(l, "Ajouter à la file")).color(Color32::WHITE)).fill(accent())).clicked() {
                        submit = Some(Tab::Concat);
                    }
                }
                Tab::Mux => {
                    ui.label(tr(l, "Ajoute une piste audio à une vidéo (la vidéo n'est pas réencodée)."));
                    pick_one(ui, l, tr(l, "Vidéo :"), &mut self.tools.mux_video, MEDIA_EXT);
                    pick_one(ui, l, tr(l, "Audio :"), &mut self.tools.mux_audio, MEDIA_EXT);
                    ui.horizontal(|ui| {
                        ui.label(tr(l, "Nom de sortie :"));
                        ui.add(egui::TextEdit::singleline(&mut self.tools.mux_name).desired_width(200.0));
                    });
                    let ok = self.tools.mux_video.is_some() && self.tools.mux_audio.is_some();
                    if ui.add_enabled(ok, egui::Button::new(RichText::new(tr(l, "Ajouter à la file")).color(Color32::WHITE)).fill(accent())).clicked() {
                        submit = Some(Tab::Mux);
                    }
                }
                Tab::Mix => {
                    ui.label(tr(l, "Mélange plusieurs pistes audio en une seule (durée de la plus longue)."));
                    file_list(ui, l, "mf", &mut self.tools.mix_files, MEDIA_EXT);
                    ui.horizontal(|ui| {
                        ui.label(tr(l, "Profil audio :"));
                        let audio: Vec<Profile> = self.profiles.iter().filter(|p| p.is_audio_only()).cloned().collect();
                        let sel = audio.iter().find(|p| p.id == self.tools.mix_profile).map(|p| plabel(l, p)).unwrap_or_default();
                        egui::ComboBox::from_id_salt("mixp").selected_text(sel).show_ui(ui, |ui| {
                            for p in &audio {
                                if ui.selectable_label(p.id == self.tools.mix_profile, plabel(l, p)).clicked() {
                                    self.tools.mix_profile = p.id.clone();
                                }
                            }
                        });
                    });
                    ui.horizontal(|ui| {
                        ui.label(tr(l, "Nom de sortie :"));
                        ui.add(egui::TextEdit::singleline(&mut self.tools.mix_name).desired_width(200.0));
                    });
                    if ui.add_enabled(self.tools.mix_files.len() >= 2, egui::Button::new(RichText::new(tr(l, "Ajouter à la file")).color(Color32::WHITE)).fill(accent())).clicked() {
                        submit = Some(Tab::Mix);
                    }
                }
                Tab::Images => crate::batch::images_tab(&mut self.tools.img, ui, l),
                Tab::Rename => {
                    ui.label(tr(l, "Renomme des fichiers sur le disque (l'extension est conservée)."));
                    file_list(ui, l, "rf", &mut self.tools.ren_files, &[]);
                    let r = &mut self.tools.rule;
                    ui.horizontal(|ui| {
                        ui.label(tr(l, "Nouveau nom (remplace tout) :"));
                        ui.add(egui::TextEdit::singleline(&mut r.new_base).hint_text(tr(l, "ex. unautrenom")).desired_width(220.0));
                    });
                    egui::Grid::new("rule").num_columns(4).spacing([8.0, 6.0]).show(ui, |ui| {
                        ui.label(tr(l, "Rechercher :"));
                        ui.add(egui::TextEdit::singleline(&mut r.find).desired_width(120.0));
                        ui.label(tr(l, "Remplacer par :"));
                        ui.add(egui::TextEdit::singleline(&mut r.replace).desired_width(120.0));
                        ui.end_row();
                        ui.label(tr(l, "Préfixe :"));
                        ui.add(egui::TextEdit::singleline(&mut r.prefix).desired_width(120.0));
                        ui.label(tr(l, "Suffixe :"));
                        ui.add(egui::TextEdit::singleline(&mut r.suffix).desired_width(120.0));
                        ui.end_row();
                    });
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut r.number, tr(l, "Numéroter"));
                        if r.number {
                            ui.label(tr(l, "départ"));
                            ui.add(egui::DragValue::new(&mut r.start).range(0..=99999));
                            ui.label(tr(l, "chiffres"));
                            ui.add(egui::DragValue::new(&mut r.digits).range(1..=8));
                            ui.checkbox(&mut r.number_at_end, tr(l, "à la fin"));
                            ui.checkbox(&mut r.no_sep, tr(l, "sans séparateur"));
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label(tr(l, "Casse :"));
                        for (m, label) in [(CaseMode::Keep, "inchangée"), (CaseMode::Lower, "minuscules"), (CaseMode::Upper, "MAJUSCULES"), (CaseMode::Title, "Titre")] {
                            if ui.selectable_label(r.case == m, tr(l, label)).clicked() {
                                r.case = m;
                            }
                        }
                    });
                    let steps = plan(&self.tools.ren_files, &self.tools.rule);
                    let changing = steps.iter().filter(|s| s.conflict.is_none() && s.from != s.to).count();
                    egui::ScrollArea::vertical().id_salt("prev").max_height(130.0).show(ui, |ui| {
                        for s in steps.iter().take(60) {
                            let line = format!("{}  →  {}", file_name(&s.from), file_name(&s.to));
                            match &s.conflict {
                                Some(c) => ui.colored_label(red(), format!("{line}   ({})", tr(l, c))),
                                None => ui.label(RichText::new(line).small()),
                            };
                        }
                    });
                    ui.checkbox(&mut self.tools.ren_confirm, tr(l, "Je confirme : le renommage est définitif."));
                    if ui.add_enabled(self.tools.ren_confirm && changing > 0, egui::Button::new(RichText::new(trf(l, "Renommer ({n})", &[("{n}", changing.to_string())])).color(Color32::WHITE)).fill(accent())).clicked() {
                        let (done, errs) = apply(&steps);
                        let renamed: Vec<PathBuf> = steps.iter().map(|s| if s.conflict.is_none() && s.from != s.to && errs.is_empty() { s.to.clone() } else { s.from.clone() }).collect();
                        self.tools.ren_files = renamed;
                        self.tools.ren_confirm = false;
                        self.tools.msg = if errs.is_empty() {
                            trf(l, "{n} fichier(s) renommé(s).", &[("{n}", done.to_string())])
                        } else {
                            format!("{} {}", trf(l, "{n} fichier(s) renommé(s).", &[("{n}", done.to_string())]), errs.join(" | "))
                        };
                        for e in errs {
                            ff_profiles::log(&format!("ERREUR renommage : {e}"));
                        }
                    }
                }
            }
            if !self.tools.msg.is_empty() {
                ui.add_space(4.0);
                ui.label(RichText::new(&self.tools.msg).weak());
            }
        });
        self.show_tools = open;

        if let Some(tab) = submit {
            let cur = self.profiles.get(self.selected).cloned().unwrap_or_else(|| builtin_by_id("mp4").unwrap());
            match tab {
                Tab::Concat => {
                    let files = std::mem::take(&mut self.tools.concat_files);
                    let first = files[0].clone();
                    let (profile, copy) = if self.tools.concat_copy {
                        let mut p = builtin_by_id("mkv").unwrap();
                        p.label = "Fusion sans réencodage".into();
                        p.extension = ext_of(&first);
                        (p, true)
                    } else {
                        (cur, false)
                    };
                    let title = trf(l, "Fusion de {n} fichiers", &[("{n}", files.len().to_string())]);
                    let name = self.tools.concat_name.clone();
                    self.add_tool_item(first, title, JobKind::Concat { inputs: files, copy }, profile, &name);
                }
                Tab::Mux => {
                    if let (Some(v), Some(a)) = (self.tools.mux_video.take(), self.tools.mux_audio.take()) {
                        let mut p = builtin_by_id("mkv").unwrap();
                        p.label = "Ajout de piste audio".into();
                        p.extension = ext_of(&v);
                        let title = format!("{} {}", tr(l, "Ajout audio :"), file_name(&v));
                        let name = self.tools.mux_name.clone();
                        self.add_tool_item(v, title, JobKind::Mux { audio: a }, p, &name);
                    }
                }
                Tab::Mix => {
                    let files = std::mem::take(&mut self.tools.mix_files);
                    let profile = self.profiles.iter().find(|p| p.id == self.tools.mix_profile && p.is_audio_only()).cloned().unwrap_or_else(|| builtin_by_id("mp3").unwrap());
                    let title = trf(l, "Mixage de {n} pistes", &[("{n}", files.len().to_string())]);
                    let name = self.tools.mix_name.clone();
                    self.add_tool_item(files[0].clone(), title, JobKind::Mix { inputs: files }, profile, &name);
                }
                Tab::Rename | Tab::Images => {}
            }
            self.tools.msg = tr(l, "Ajouté à la file : lancez la conversion depuis la fenêtre principale.").to_string();
        }
        let _ = reveal;
    }
}
