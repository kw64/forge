//! Fenêtre « Renommer en lot » de FileForge (règles + filtres + aperçu en direct, façon ReNamer). v0.11.0
use crate::pane::Loc;
use crate::rename::*;
use crate::App;
use eframe::egui::{self, Color32, RichText};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

type Pending = Arc<Mutex<Option<(bool, Result<Vec<(PathBuf, PathBuf)>, String>)>>>;

struct RFile {
    info: FileInfo,
    checked: bool,
}

pub struct RenameUi {
    pub open: bool,
    files: Vec<RFile>,
    rules: Vec<RuleItem>,
    sel: Option<usize>,
    filters: Filters,
    prev: Vec<Preview>,
    sig: String,
    ver: u64,
    presets: BTreeMap<String, Vec<RuleItem>>,
    preset_name: String,
    undo: Vec<(PathBuf, PathBuf)>,
    sort: usize,
    desc: bool,
    recurse: bool,
    msg: String,
    /// Résultat du renommage en tâche de fond (annulation ?, paires effectuées).
    pending: Option<Pending>,
}

fn presets_path() -> PathBuf {
    ff_profiles::data_dir().join("rename_presets.json")
}

impl Default for RenameUi {
    fn default() -> Self {
        let presets = std::fs::read_to_string(presets_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        RenameUi { open: false, files: vec![], rules: vec![], sel: None, filters: Filters::default(), prev: vec![], sig: String::new(), ver: 0, presets, preset_name: String::new(), undo: vec![], sort: 0, desc: false, recurse: false, msg: String::new(), pending: None }
    }
}

impl RenameUi {
    fn add_paths(&mut self, paths: impl IntoIterator<Item = PathBuf>) {
        for p in paths {
            if self.files.iter().any(|f| f.info.path == p) {
                continue;
            }
            if let Some(info) = FileInfo::from_path(&p) {
                self.files.push(RFile { info, checked: true });
            }
        }
        self.ver += 1;
    }

    fn add_folder(&mut self, dir: &std::path::Path) {
        let mut found = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            if let Ok(rd) = std::fs::read_dir(&d) {
                for e in rd.flatten() {
                    let p = e.path();
                    if self.recurse && p.is_dir() {
                        stack.push(p);
                    } else {
                        found.push(p);
                    }
                }
            }
            if !self.recurse {
                break;
            }
        }
        found.sort_by(|a, b| natural_cmp(&a.to_string_lossy(), &b.to_string_lossy()));
        self.add_paths(found);
    }

    fn sort_files(&mut self) {
        let k = self.sort;
        self.files.sort_by(|a, b| {
            let (a, b) = (&a.info, &b.info);
            match k {
                1 => a.mtime.cmp(&b.mtime),
                2 => a.size.cmp(&b.size),
                3 => split_name(&a.name, a.is_dir).1.to_lowercase().cmp(&split_name(&b.name, b.is_dir).1.to_lowercase()).then_with(|| natural_cmp(&a.name, &b.name)),
                _ => natural_cmp(&a.path.to_string_lossy(), &b.path.to_string_lossy()),
            }
        });
        if self.desc {
            self.files.reverse();
        }
        self.ver += 1;
    }

    fn active(&self) -> Vec<bool> {
        self.files.iter().map(|f| f.checked && self.filters.pass(&f.info.name, f.info.is_dir)).collect()
    }

    fn save_presets(&self) {
        if let Some(d) = presets_path().parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if let Ok(s) = serde_json::to_string_pretty(&self.presets) {
            let _ = std::fs::write(presets_path(), s);
        }
    }

    fn recompute(&mut self, en: bool) {
        let sig = format!("{:?}|{:?}|{}|{}", self.rules, self.filters, self.ver, self.files.iter().map(|f| if f.checked { '1' } else { '0' }).collect::<String>());
        if sig != self.sig {
            let infos: Vec<FileInfo> = self.files.iter().map(|f| f.info.clone()).collect();
            self.prev = preview(&infos, &self.active(), &self.rules, en);
            self.sig = sig;
        }
    }
}

/// Cellule de tableau à largeur fixe, texte aligné à gauche.
fn cell(ui: &mut egui::Ui, w: f32, label: egui::Label) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, 18.0), egui::Sense::hover());
    let mut c = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::left_to_right(egui::Align::Center)));
    c.add(label);
}

fn place_ui(ui: &mut egui::Ui, salt: &str, p: &mut Place, en: bool, allow_replace: bool) {
    let t = |fr: &'static str, e: &'static str| if en { e } else { fr };
    let cur = match p {
        Place::Prefix => t("Début", "Start"),
        Place::Suffix => t("Fin", "End"),
        Place::Position(_) => t("Position", "Position"),
        Place::After(_) => t("Après le texte", "After text"),
        Place::Before(_) => t("Avant le texte", "Before text"),
        Place::Replace => t("Remplace le nom", "Replace name"),
    };
    egui::ComboBox::from_id_salt(salt).selected_text(cur).show_ui(ui, |ui| {
        if ui.selectable_label(matches!(p, Place::Prefix), t("Début", "Start")).clicked() {
            *p = Place::Prefix;
        }
        if ui.selectable_label(matches!(p, Place::Suffix), t("Fin", "End")).clicked() {
            *p = Place::Suffix;
        }
        if ui.selectable_label(matches!(p, Place::Position(_)), t("Position", "Position")).clicked() {
            *p = Place::Position(1);
        }
        if ui.selectable_label(matches!(p, Place::After(_)), t("Après le texte", "After text")).clicked() {
            *p = Place::After(String::new());
        }
        if ui.selectable_label(matches!(p, Place::Before(_)), t("Avant le texte", "Before text")).clicked() {
            *p = Place::Before(String::new());
        }
        if allow_replace && ui.selectable_label(matches!(p, Place::Replace), t("Remplace le nom", "Replace name")).clicked() {
            *p = Place::Replace;
        }
    });
    match p {
        Place::Position(n) => {
            ui.add(egui::DragValue::new(n).range(1..=999));
        }
        Place::After(s) | Place::Before(s) => {
            ui.add(egui::TextEdit::singleline(s).desired_width(120.0));
        }
        _ => {}
    }
}

fn which_ui(ui: &mut egui::Ui, salt: &str, w: &mut Which, en: bool) {
    let name = |w: Which| match (w, en) {
        (Which::All, false) => "Toutes",
        (Which::All, true) => "All",
        (Which::First, false) => "Première",
        (Which::First, true) => "First",
        (Which::Last, false) => "Dernière",
        (Which::Last, true) => "Last",
    };
    egui::ComboBox::from_id_salt(salt).selected_text(name(*w)).show_ui(ui, |ui| {
        for v in [Which::All, Which::First, Which::Last] {
            ui.selectable_value(w, v, name(v));
        }
    });
}

fn rule_editor(ui: &mut egui::Ui, r: &mut RuleItem, en: bool) {
    let t = |fr: &'static str, e: &'static str| if en { e } else { fr };
    let is_ext = matches!(r.kind, Kind::Extension { .. });
    if !is_ext {
        ui.horizontal(|ui| {
            ui.label(t("Appliquer à :", "Apply to:"));
            for (s, fr, e) in [(Scope::Name, "Nom", "Name"), (Scope::Ext, "Extension", "Extension"), (Scope::Both, "Nom + ext.", "Name + ext.")] {
                ui.selectable_value(&mut r.scope, s, t(fr, e));
            }
        });
    }
    match &mut r.kind {
        Kind::Insert { text, place, from_right } => {
            ui.horizontal(|ui| {
                ui.label(t("Texte :", "Text:"));
                ui.add(egui::TextEdit::singleline(text).desired_width(200.0));
                place_ui(ui, "ins_pl", place, en, false);
                if matches!(place, Place::Position(_)) {
                    ui.checkbox(from_right, t("depuis la droite", "from right"));
                }
            });
        }
        Kind::Delete { from, from_right, mode, count, until } => {
            ui.horizontal(|ui| {
                ui.label(t("À partir de la position", "From position"));
                ui.add(egui::DragValue::new(from).range(1..=999));
                ui.checkbox(from_right, t("depuis la droite", "from right"));
            });
            ui.horizontal(|ui| {
                ui.selectable_value(mode, DelMode::Count, t("Nombre de caractères", "Number of chars"));
                ui.selectable_value(mode, DelMode::Until, t("Jusqu'au texte", "Until text"));
                ui.selectable_value(mode, DelMode::End, t("Jusqu'au bout", "To the end"));
            });
            match mode {
                DelMode::Count => {
                    ui.add(egui::DragValue::new(count).range(0..=999));
                }
                DelMode::Until => {
                    ui.add(egui::TextEdit::singleline(until).desired_width(160.0));
                }
                DelMode::End => {}
            }
        }
        Kind::Remove { text, case, which } => {
            ui.horizontal(|ui| {
                ui.label(t("Texte :", "Text:"));
                ui.add(egui::TextEdit::singleline(text).desired_width(200.0));
                which_ui(ui, "rm_w", which, en);
                ui.checkbox(case, t("Respecter la casse", "Match case"));
            });
        }
        Kind::Replace { find, with, case, regex, which } => {
            egui::Grid::new("rep").num_columns(2).show(ui, |ui| {
                ui.label(t("Chercher :", "Find:"));
                ui.add(egui::TextEdit::singleline(find).desired_width(260.0));
                ui.end_row();
                ui.label(t("Remplacer par :", "Replace with:"));
                ui.add(egui::TextEdit::singleline(with).desired_width(260.0));
                ui.end_row();
            });
            ui.horizontal(|ui| {
                which_ui(ui, "rp_w", which, en);
                ui.checkbox(case, t("Respecter la casse", "Match case"));
                ui.checkbox(regex, t("Expression régulière ($1, $2…)", "Regular expression ($1, $2…)"));
            });
        }
        Kind::Rearrange { delims, pattern } => {
            egui::Grid::new("rea").num_columns(2).show(ui, |ui| {
                ui.label(t("Séparateurs :", "Delimiters:"));
                ui.add(egui::TextEdit::singleline(delims).desired_width(160.0));
                ui.end_row();
                ui.label(t("Modèle :", "Pattern:"));
                ui.add(egui::TextEdit::singleline(pattern).desired_width(260.0));
                ui.end_row();
            });
            ui.label(RichText::new(t("$1, $2… = morceaux ; $-1 = dernier ; $0 = nom entier. Ex : « $2 - $1 »", "$1, $2… = parts; $-1 = last; $0 = whole name. E.g. “$2 - $1”")).small().weak());
        }
        Kind::Extension { mode, text } => {
            ui.horizontal(|ui| {
                ui.selectable_value(mode, ExtMode::Lower, t("minuscules", "lowercase"));
                ui.selectable_value(mode, ExtMode::Upper, t("MAJUSCULES", "UPPERCASE"));
                ui.selectable_value(mode, ExtMode::Set, t("Fixer", "Set"));
                ui.selectable_value(mode, ExtMode::AddIfMissing, t("Ajouter si absente", "Add if missing"));
                ui.selectable_value(mode, ExtMode::Remove, t("Supprimer", "Remove"));
            });
            if matches!(mode, ExtMode::Set | ExtMode::AddIfMissing) {
                ui.add(egui::TextEdit::singleline(text).desired_width(100.0).hint_text("jpg"));
            }
        }
        Kind::Strip { letters, digits, symbols, brackets, accents, spaces, custom } => {
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(letters, t("Lettres", "Letters"));
                ui.checkbox(digits, t("Chiffres", "Digits"));
                ui.checkbox(symbols, t("Symboles", "Symbols"));
                ui.checkbox(brackets, t("Parenthèses/crochets + contenu", "Brackets + content"));
                ui.checkbox(accents, t("Accents", "Accents"));
                ui.checkbox(spaces, t("Espaces multiples/bords", "Extra spaces"));
            });
            ui.horizontal(|ui| {
                ui.label(t("Caractères à retirer :", "Characters to remove:"));
                ui.add(egui::TextEdit::singleline(custom).desired_width(160.0));
            });
        }
        Kind::Case { mode, exceptions } => {
            ui.horizontal(|ui| {
                ui.selectable_value(mode, CaseMode::Lower, t("minuscules", "lower"));
                ui.selectable_value(mode, CaseMode::Upper, t("MAJUSCULES", "UPPER"));
                ui.selectable_value(mode, CaseMode::Title, t("Titre", "Title"));
                ui.selectable_value(mode, CaseMode::Sentence, t("Phrase", "Sentence"));
                ui.selectable_value(mode, CaseMode::Invert, t("Inverser", "Invert"));
            });
            if *mode == CaseMode::Title {
                ui.horizontal(|ui| {
                    ui.label(t("Mots en minuscules :", "Lowercase words:"));
                    ui.add(egui::TextEdit::singleline(exceptions).desired_width(280.0));
                });
            }
        }
        Kind::Serialize { start, step, pad, place, sep, per_folder } => {
            ui.horizontal(|ui| {
                ui.label(t("Début", "Start"));
                ui.add(egui::DragValue::new(start));
                ui.label(t("Pas", "Step"));
                ui.add(egui::DragValue::new(step));
                ui.label(t("Chiffres", "Digits"));
                ui.add(egui::DragValue::new(pad).range(1..=12));
            });
            ui.horizontal(|ui| {
                place_ui(ui, "se_pl", place, en, true);
                ui.label(t("Séparateur :", "Separator:"));
                ui.add(egui::TextEdit::singleline(sep).desired_width(40.0));
                ui.checkbox(per_folder, t("Recommencer par dossier", "Restart per folder"));
            });
        }
        Kind::Date { format, place, source } => {
            ui.horizontal(|ui| {
                ui.label(t("Format :", "Format:"));
                ui.add(egui::TextEdit::singleline(format).desired_width(150.0));
                ui.selectable_value(source, DateSrc::Modified, t("Modification", "Modified"));
                ui.selectable_value(source, DateSrc::Created, t("Création", "Created"));
                ui.selectable_value(source, DateSrc::Now, t("Maintenant", "Now"));
            });
            ui.horizontal(|ui| {
                place_ui(ui, "da_pl", place, en, false);
                ui.label(RichText::new("yyyy yy mm dd hh nn ss").small().weak());
            });
        }
    }
}

enum Act {
    AddFiles,
    AddFolder,
    FromPane,
    Rename,
    Undo,
    Clear,
    RemoveUnchecked,
}

impl App {
    /// Renommage (ou annulation) en tâche de fond : l'interface reste utilisable.
    fn rename_job(&mut self, pairs: Vec<(PathBuf, PathBuf)>, undo: bool) {
        if self.rename.pending.is_some() || self.jobs.running("rename") {
            return;
        }
        let en = self.en();
        let slot: Pending = Arc::new(Mutex::new(None));
        self.rename.pending = Some(slot.clone());
        let label = if undo { if en { "Undoing rename" } else { "Annulation du renommage" } } else if en { "Renaming files" } else { "Renommage des fichiers" };
        self.jobs.spawn("rename", label, move |_job| {
            let r = execute(&pairs);
            let msg = r.as_ref().map(|d| format!("{} {}", d.len(), if en { "file(s) renamed." } else { "fichier(s) renommé(s)." })).map_err(|e| e.clone());
            *slot.lock().unwrap() = Some((undo, r));
            msg
        });
    }

    /// Applique le résultat d'un renommage terminé (appelé à chaque image).
    pub fn rename_apply_pending(&mut self) {
        let Some(slot) = self.rename.pending.clone() else { return };
        let Some((undo, res)) = slot.lock().unwrap().take() else { return };
        self.rename.pending = None;
        let en = self.en();
        match res {
            Ok(done) => {
                let n = done.len();
                let map: std::collections::HashMap<PathBuf, PathBuf> = done.iter().cloned().collect();
                for f in &mut self.rename.files {
                    if let Some(d) = map.get(&f.info.path) {
                        f.info.path = d.clone();
                        f.info.name = d.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                    }
                }
                if undo {
                    self.rename.undo.clear();
                    self.rename.msg = if en { "Rename undone.".into() } else { "Renommage annulé.".into() };
                } else {
                    self.rename.undo = done;
                    self.rename.msg = format!("{n} {}", if en { "file(s) renamed." } else { "fichier(s) renommé(s)." });
                }
                self.rename.ver += 1;
            }
            Err(e) => self.rename.msg = format!("{} {e}", if en { "Failed (rolled back):" } else { "Échec (tout a été annulé) :" }),
        }
    }

    pub fn open_rename(&mut self) {
        let a = self.active;
        if self.rename.files.is_empty() {
            self.rename_from_pane(a);
        }
        self.rename.open = true;
    }

    fn rename_from_pane(&mut self, a: usize) {
        let Loc::Disk(dir) = self.panes[a].loc.clone() else { return };
        let p = &self.panes[a];
        let names: Vec<String> = if p.sel.is_empty() { p.rows.iter().filter(|r| !r.is_parent).map(|r| r.full.clone()).collect() } else { p.rows.iter().filter(|r| !r.is_parent && p.sel.contains(&r.full)).map(|r| r.full.clone()).collect() };
        self.rename.add_paths(names.into_iter().map(|n| dir.join(n)));
    }

    pub fn rename_window(&mut self, ctx: &egui::Context) {
        if !self.rename.open {
            return;
        }
        let en = self.en();
        let t = |fr: &'static str, e: &'static str| if en { e } else { fr };
        // Glisser-déposer de fichiers sur la fenêtre.
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        if !dropped.is_empty() {
            self.rename.add_paths(dropped);
        }
        self.rename.recompute(en);
        let pal = ff_kit::theme::current();
        let busy = self.rename.pending.is_some() || self.jobs.running("rename");
        let mut open = true;
        let mut act: Option<Act> = None;
        let mut new_rule: Option<usize> = None;
        let mut move_rule: Option<(usize, i32)> = None;
        let mut del_rule = false;
        let mut dup_rule = false;
        let mut save_preset = false;
        let mut load_preset: Option<String> = None;
        let mut del_preset = false;
        egui::Window::new(t("Renommer en lot", "Batch rename")).open(&mut open).default_size([860.0, 700.0]).resizable(true).show(ctx, |ui| {
            ff_kit::help::button(ui, "fileforge", "renommage", "?");
            let r = &mut self.rename;
            // Barre d'actions.
            ui.horizontal_wrapped(|ui| {
                if ui.button(t("Ajouter fichiers…", "Add files…")).clicked() {
                    act = Some(Act::AddFiles);
                }
                if ui.button(t("Ajouter dossier…", "Add folder…")).clicked() {
                    act = Some(Act::AddFolder);
                }
                ui.checkbox(&mut r.recurse, t("sous-dossiers", "subfolders"));
                if ui.button(t("Depuis le panneau", "From pane")).clicked() {
                    act = Some(Act::FromPane);
                }
                if ui.button(t("Vider", "Clear")).clicked() {
                    act = Some(Act::Clear);
                }
                if ui.button(t("Retirer les décochés", "Remove unchecked")).clicked() {
                    act = Some(Act::RemoveUnchecked);
                }
            });
            ui.separator();
            // Règles.
            ui.horizontal(|ui| {
                ui.menu_button(RichText::new(format!("+ {}", t("Ajouter une règle", "Add rule"))).strong(), |ui| {
                    for (i, (fr, e)) in TYPES.iter().enumerate() {
                        if ui.button(if en { *e } else { *fr }).clicked() {
                            new_rule = Some(i);
                            ui.close();
                        }
                    }
                });
                let has = r.sel.is_some();
                ui.add_enabled_ui(has, |ui| {
                    if ui.button(t("Dupliquer", "Duplicate")).clicked() {
                        dup_rule = true;
                    }
                    if ui.button(t("Retirer", "Remove")).clicked() {
                        del_rule = true;
                    }
                    if ui.button("▲").clicked() {
                        move_rule = r.sel.map(|i| (i, -1));
                    }
                    if ui.button("▼").clicked() {
                        move_rule = r.sel.map(|i| (i, 1));
                    }
                });
                ui.separator();
                egui::ComboBox::from_id_salt("presets").selected_text(t("Préréglages", "Presets")).show_ui(ui, |ui| {
                    for k in r.presets.keys() {
                        if ui.selectable_label(false, k).clicked() {
                            load_preset = Some(k.clone());
                        }
                    }
                });
                ui.add(egui::TextEdit::singleline(&mut r.preset_name).desired_width(110.0).hint_text(t("nom", "name")));
                if ui.add_enabled(!r.preset_name.trim().is_empty() && !r.rules.is_empty(), egui::Button::new(t("Enregistrer", "Save"))).clicked() {
                    save_preset = true;
                }
                if ui.add_enabled(r.presets.contains_key(r.preset_name.trim()), egui::Button::new(t("Supprimer", "Delete"))).clicked() {
                    del_preset = true;
                }
            });
            egui::ScrollArea::vertical().id_salt("rules").max_height(130.0).auto_shrink([false, true]).show(ui, |ui| {
                if r.rules.is_empty() {
                    ui.label(RichText::new(t("Aucune règle : ajoutez-en une (Insérer, Remplacer, Numéroter…).", "No rules: add one (Insert, Replace, Serialize…).")).weak());
                }
                for i in 0..r.rules.len() {
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut r.rules[i].enabled, "");
                        let label = format!("{}  {}", i + 1, describe(&r.rules[i], en));
                        if ui.add(egui::Button::selectable(r.sel == Some(i), label).wrap_mode(egui::TextWrapMode::Truncate)).clicked() {
                            r.sel = Some(i);
                        }
                    });
                }
            });
            if let Some(i) = r.sel.filter(|i| *i < r.rules.len()) {
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    rule_editor(ui, &mut r.rules[i], en);
                });
            }
            ui.collapsing(t("Filtres", "Filters"), |ui| {
                egui::Grid::new("filt").num_columns(2).show(ui, |ui| {
                    ui.label(t("Inclure :", "Include:"));
                    ui.add(egui::TextEdit::singleline(&mut r.filters.include).desired_width(300.0).hint_text("*.jpg;*.png"));
                    ui.end_row();
                    ui.label(t("Exclure :", "Exclude:"));
                    ui.add(egui::TextEdit::singleline(&mut r.filters.exclude).desired_width(300.0).hint_text("thumb;*.tmp"));
                    ui.end_row();
                });
                ui.horizontal(|ui| {
                    ui.checkbox(&mut r.filters.files, t("Fichiers", "Files"));
                    ui.checkbox(&mut r.filters.folders, t("Dossiers", "Folders"));
                    ui.checkbox(&mut r.filters.regex, t("Expression régulière", "Regular expression"));
                });
            });
            ui.horizontal(|ui| {
                ui.label(t("Trier :", "Sort:"));
                let names = [t("Nom", "Name"), t("Date", "Date"), t("Taille", "Size"), t("Extension", "Extension")];
                egui::ComboBox::from_id_salt("rsort").selected_text(names[r.sort]).show_ui(ui, |ui| {
                    for (i, n) in names.iter().enumerate() {
                        ui.selectable_value(&mut r.sort, i, *n);
                    }
                });
                ui.checkbox(&mut r.desc, t("décroissant", "descending"));
                if ui.button(t("Appliquer le tri", "Apply sort")).clicked() {
                    r.sort_files();
                }
            });
            ui.separator();

            // Tableau des fichiers.
            let active = r.active();
            let n = r.files.len();
            let errs = r.prev.iter().zip(&active).filter(|(p, a)| **a && p.err.is_some()).count();
            let changed = r.prev.iter().zip(r.files.iter()).zip(&active).filter(|((p, f), a)| **a && p.new != f.info.name).count();
            let h = (ui.available_height() - 40.0).max(80.0);
            let w = ui.available_width();
            let (w1, w3) = ((w - 40.0) * 0.38, 120.0);
            let w2 = (w - 40.0 - w1 - w3).max(80.0);
            egui::ScrollArea::vertical().id_salt("rfiles").max_height(h).auto_shrink([false, false]).show_rows(ui, 22.0, n, |ui, range| {
                for i in range {
                    let f = &mut r.files[i];
                    let p = r.prev.get(i);
                    let act_i = active[i];
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut f.checked, "");
                        let dim = if act_i { pal.text } else { pal.weak };
                        cell(ui, w1, egui::Label::new(RichText::new(&f.info.name).color(dim)).truncate());
                        let (new, err) = p.map(|p| (p.new.as_str(), p.err.as_deref())).unwrap_or(("", None));
                        let changed = act_i && new != f.info.name;
                        let col = if err.is_some() { pal.err } else if changed { Color32::from_rgb(0xe3, 0x05, 0x13) } else { pal.weak };
                        cell(ui, w2, egui::Label::new(RichText::new(new).color(col)).truncate());
                        let state = match (act_i, err) {
                            (false, _) => t("Ignoré", "Skipped").to_string(),
                            (true, Some(e)) => e.to_string(),
                            (true, None) if changed => "OK".to_string(),
                            _ => String::new(),
                        };
                        cell(ui, w3, egui::Label::new(RichText::new(state).small().color(if err.is_some() { pal.err } else { pal.weak })).truncate());
                    });
                }
            });
            ui.separator();
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{n} {} · {changed} {} · {errs} {}", t("fichier(s)", "file(s)"), t("à renommer", "to rename"), t("erreur(s)", "error(s)"))).small());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add_enabled(changed > 0 && errs == 0 && !busy, egui::Button::new(RichText::new(if busy { t("Renommage…", "Renaming…") } else { t("Renommer", "Rename") }).strong().color(Color32::WHITE)).fill(Color32::from_rgb(0x00, 0x97, 0x3a))).clicked() {
                        act = Some(Act::Rename);
                    }
                    if ui.add_enabled(!r.undo.is_empty() && !busy, egui::Button::new(t("Annuler le dernier", "Undo last"))).clicked() {
                        act = Some(Act::Undo);
                    }
                    ui.label(RichText::new(r.msg.clone()).small().color(pal.weak));
                });
            });
        });

        // --- actions différées ---
        if let Some(i) = new_rule {
            self.rename.rules.push(default_rule(i));
            self.rename.sel = Some(self.rename.rules.len() - 1);
        }
        if dup_rule {
            if let Some(i) = self.rename.sel.filter(|i| *i < self.rename.rules.len()) {
                let c = self.rename.rules[i].clone();
                self.rename.rules.insert(i + 1, c);
                self.rename.sel = Some(i + 1);
            }
        }
        if del_rule {
            if let Some(i) = self.rename.sel.filter(|i| *i < self.rename.rules.len()) {
                self.rename.rules.remove(i);
                self.rename.sel = if self.rename.rules.is_empty() { None } else { Some(i.min(self.rename.rules.len() - 1)) };
            }
        }
        if let Some((i, d)) = move_rule {
            let j = i as i32 + d;
            if j >= 0 && (j as usize) < self.rename.rules.len() {
                self.rename.rules.swap(i, j as usize);
                self.rename.sel = Some(j as usize);
            }
        }
        if save_preset {
            let n = self.rename.preset_name.trim().to_string();
            let rules = self.rename.rules.clone();
            self.rename.presets.insert(n, rules);
            self.rename.save_presets();
        }
        if let Some(k) = load_preset {
            if let Some(rs) = self.rename.presets.get(&k).cloned() {
                self.rename.rules = rs;
                self.rename.sel = None;
                self.rename.preset_name = k;
            }
        }
        if del_preset {
            let n = self.rename.preset_name.trim().to_string();
            self.rename.presets.remove(&n);
            self.rename.save_presets();
        }
        match act {
            Some(Act::AddFiles) => {
                if let Some(f) = rfd::FileDialog::new().pick_files() {
                    self.rename.add_paths(f);
                }
            }
            Some(Act::AddFolder) => {
                if let Some(d) = rfd::FileDialog::new().pick_folder() {
                    self.rename.add_folder(&d);
                }
            }
            Some(Act::FromPane) => {
                let a = self.active;
                self.rename_from_pane(a);
            }
            Some(Act::Clear) => {
                self.rename.files.clear();
                self.rename.ver += 1;
            }
            Some(Act::RemoveUnchecked) => {
                self.rename.files.retain(|f| f.checked);
                self.rename.ver += 1;
            }
            Some(Act::Rename) => {
                let active = self.rename.active();
                let pairs: Vec<(PathBuf, PathBuf)> = self.rename.files.iter().zip(&self.rename.prev).zip(&active).filter(|((f, p), a)| **a && p.new != f.info.name && p.err.is_none()).map(|((f, p), _)| (f.info.path.clone(), f.info.path.with_file_name(&p.new))).collect();
                self.rename_job(pairs, false);
            }
            Some(Act::Undo) => {
                let back: Vec<(PathBuf, PathBuf)> = self.rename.undo.iter().map(|(s, d)| (d.clone(), s.clone())).collect();
                self.rename_job(back, true);
            }
            None => {}
        }
        if !open {
            self.rename.open = false;
        }
    }
}
