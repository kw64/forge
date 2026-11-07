//! AutoForge v0.1.0 — rangement, tri et identification automatiques de fichiers (module IA indépendant).
//! Niveau 1 : règles (EXIF, tags, nom, date) · Niveau 2 : IA locale LM Studio · Identification Internet/DAT.
//! Principe : l'application propose un PLAN (fichier → destination) que l'utilisateur valide ; rien n'est supprimé.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui::{self, Color32, RichText};
use ff_auto::cfg::{self, Config};
use ff_auto::job::Jobs;
use ff_auto::plan::{self, Item, Origin};
use ff_auto::rules::{self, Rule};
use ff_auto::scan::{self, Facts, Kind};
use ff_auto::{cmd, dat, llm, web};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Rules,
    Ai,
    Command,
    Ident,
    Settings,
}

/// Résultats renvoyés par les tâches de fond.
enum Out {
    Plan(Vec<Item>, String),
    Search(Vec<Facts>),
    Cmd(cmd::Command),
    Models(Vec<String>),
}

struct Row {
    item: Item,
    text: String,
}

struct App {
    cfg: Config,
    sett: ff_profiles::Settings,
    tab: Tab,
    jobs: Jobs,
    inbox: Arc<Mutex<Vec<Out>>>,
    rows: Vec<Row>,
    filter: String,
    msg: String,
    cmd_text: String,
    cmd: Option<cmd::Command>,
    results: Vec<Facts>,
    models: Vec<String>,
    ai_photos: bool,
    ai_text: bool,
    ai_only_other: bool,
    ai_min_conf: f32,
    theme_check: std::time::Instant,
    undo_available: bool,
}

fn pick_dir(cur: &str) -> Option<String> {
    let mut d = rfd::FileDialog::new();
    if !cur.is_empty() {
        d = d.set_directory(cur);
    }
    d.pick_folder().map(|p| p.display().to_string())
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, start: Option<PathBuf>) -> App {
        let sett = ff_profiles::load();
        ff_kit::theme::apply(&cc.egui_ctx, ff_kit::theme::Style::from_id(&sett.theme), sett.dark);
        let mut cfg = cfg::load();
        if let Some(s) = start {
            cfg.source = s.display().to_string();
            if cfg.dest.is_empty() {
                cfg.dest = cfg.source.clone();
            }
        }
        App { cfg, sett, tab: Tab::Rules, jobs: Jobs::default(), inbox: Arc::default(), rows: vec![], filter: String::new(), msg: String::new(), cmd_text: String::new(), cmd: None, results: vec![], models: vec![], ai_photos: true, ai_text: true, ai_only_other: false, ai_min_conf: 0.5, theme_check: std::time::Instant::now(), undo_available: plan::last_journal().is_some() }
    }

    fn save(&mut self) {
        if let Err(e) = cfg::save(&self.cfg) {
            self.msg = format!("Enregistrement impossible : {e}");
        }
    }

    fn src(&self) -> PathBuf {
        PathBuf::from(&self.cfg.source)
    }

    fn dest(&self) -> PathBuf {
        PathBuf::from(if self.cfg.dest.is_empty() { &self.cfg.source } else { &self.cfg.dest })
    }

    fn need_src(&mut self) -> bool {
        if !self.src().is_dir() {
            self.msg = "Choisissez d'abord un dossier source valide.".into();
            return false;
        }
        true
    }

    fn set_plan(&mut self, items: Vec<Item>) {
        let base = self.src();
        let _ = base;
        self.rows = items.into_iter().map(|item| Row { text: item.dst.display().to_string(), item }).collect();
    }

    /// Lance une tâche de fond qui publie un résultat dans `inbox`.
    fn spawn(&mut self, key: &'static str, label: &str, f: impl FnOnce(&ff_auto::job::Job) -> Result<Out, String> + Send + 'static) {
        let inbox = self.inbox.clone();
        if !self.jobs.spawn(key, label, move |j| {
            let o = f(j)?;
            let msg = match &o {
                Out::Plan(_, m) => m.clone(),
                _ => String::new(),
            };
            inbox.lock().map(|mut v| v.push(o)).ok();
            Ok(msg)
        }) {
            self.msg = "Cette tâche est déjà en cours.".into();
        }
    }

    // ---- Lancement des tâches -------------------------------------------------------------

    fn start_rules(&mut self) {
        if !self.need_src() {
            return;
        }
        self.save();
        let (cfg, src, dest) = (self.cfg.clone(), self.src(), self.dest());
        self.spawn("rules", "Analyse et plan de tri", move |j| {
            j.set_detail("Parcours du dossier…");
            let paths = scan::walk(&src, cfg.recurse, Some(j));
            j.set_detail("Lecture des métadonnées…");
            let facts = scan::scan(&paths, true, j);
            let items = plan::build(&facts, &cfg, &dest, j);
            let n = items.len();
            Ok(Out::Plan(items, format!("{n} déplacement(s) proposé(s) sur {} fichier(s).", facts.len())))
        });
    }

    fn start_ai(&mut self) {
        if !self.need_src() {
            return;
        }
        self.save();
        let (cfg, src, dest) = (self.cfg.clone(), self.src(), self.dest());
        let (photos, text, only_other, minc) = (self.ai_photos, self.ai_text, self.ai_only_other, self.ai_min_conf);
        self.spawn("ai", "Classement par l'IA", move |j| {
            let llm = llm::Llm::new(&cfg);
            llm.models()?; // échoue vite si LM Studio est injoignable
            let paths = scan::walk(&src, cfg.recurse, Some(j));
            let facts = scan::scan(&paths, true, j);
            let todo: Vec<&Facts> = facts
                .iter()
                .filter(|f| (photos && f.kind() == Kind::Photo) || (text && f.kind() == Kind::Doc && matches!(f.ext.as_str(), "txt" | "md" | "csv" | "html" | "htm")))
                .filter(|f| !only_other || rules::first_match(&cfg.rules, f).map(|r| r.kinds.contains(&Kind::Other)).unwrap_or(true))
                .collect();
            j.reset(todo.len() as u64);
            let (mut items, mut fails) = (vec![], 0);
            for f in todo {
                j.set_detail(f.name());
                match llm::classify(&llm, f, &cfg.categories) {
                    Ok((cat, title, conf)) => {
                        let dir = rules::render(&cfg.ai_template, f, &[("categorie", cat.clone()), ("catégorie", cat.clone()), ("titre", title.clone())]);
                        items.push(Item { src: f.path.clone(), dst: dest.join(dir).join(f.name()), reason: format!("IA : {cat} ({:.0} %) « {title} »", conf * 100.0), origin: Origin::Ai, checked: conf >= minc });
                    }
                    Err(_) => fails += 1,
                }
                if !j.step(1) {
                    break;
                }
            }
            let n = items.len();
            Ok(Out::Plan(items, format!("{n} fichier(s) classé(s) par l'IA ({fails} échec(s)). Vérifiez les cases décochées (confiance faible).")))
        });
    }

    fn start_cmd_parse(&mut self) {
        self.save();
        let (cfg, text) = (self.cfg.clone(), self.cmd_text.clone());
        self.spawn("cmd_parse", "Interprétation de la commande", move |_| Ok(Out::Cmd(cmd::parse(&llm::Llm::new(&cfg), &text)?)));
    }

    fn start_cmd_run(&mut self) {
        let Some(c) = self.cmd.clone() else { return };
        if !self.need_src() {
            return;
        }
        let (cfg, src, dest) = (self.cfg.clone(), self.src(), self.dest());
        self.spawn("cmd_run", "Exécution de la commande", move |j| {
            let paths = scan::walk(&src, cfg.recurse, Some(j));
            let facts = scan::scan(&paths, true, j);
            if matches!(c.action, cmd::Action::Search) {
                let hits: Vec<Facts> = facts.into_iter().filter(|f| c.filter.matches(f)).collect();
                return Ok(Out::Search(hits));
            }
            let items = cmd::plan(&c, &facts, &dest);
            let n = items.len();
            Ok(Out::Plan(items, format!("{n} fichier(s) concerné(s) — vérifiez le plan puis appliquez.")))
        });
    }

    fn start_ident(&mut self) {
        if !self.need_src() {
            return;
        }
        self.save();
        let cfg = self.cfg.clone();
        let src = self.src();
        self.spawn("ident", "Identification (Internet / DAT)", move |j| {
            let mut index = dat::DatIndex::default();
            for p in &cfg.dat_files {
                index.load(p)?;
            }
            let llm = cfg.llm_title.then(|| llm::Llm::new(&cfg));
            let llm = llm.filter(|l| l.models().is_ok()); // IA facultative : sans serveur on se contente du nettoyage du nom
            let paths = scan::walk(&src, cfg.recurse, Some(j));
            let facts: Vec<Facts> = scan::scan(&paths, true, j).into_iter().filter(|f| matches!(f.kind(), Kind::Video | Kind::Audio | Kind::Rom)).collect();
            j.reset(facts.len() as u64);
            let (mut items, mut unknown, mut errs) = (vec![], 0, vec![]);
            for f in &facts {
                j.set_detail(f.name());
                match web::identify(f, &cfg, llm.as_ref(), &index) {
                    Ok(Some(id)) => {
                        let ext = f.path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
                        let dst = f.path.with_file_name(format!("{}{ext}", id.new_stem));
                        if dst != f.path {
                            items.push(Item { src: f.path.clone(), dst, reason: format!("{} (fiabilité {:.0} %)", id.detail, id.score * 100.0), origin: Origin::Web, checked: id.score >= 0.75 });
                        }
                    }
                    Ok(None) => unknown += 1,
                    Err(e) => {
                        if !errs.contains(&e) {
                            errs.push(e);
                        }
                    }
                }
                if !j.step(1) {
                    break;
                }
            }
            let mut m = format!("{} nom(s) proposé(s), {unknown} non identifié(s).", items.len());
            if let Some(e) = errs.first() {
                m.push_str(&format!(" Erreur : {e}"));
            }
            Ok(Out::Plan(items, m))
        });
    }

    fn start_apply(&mut self) {
        for r in &mut self.rows {
            r.item.dst = PathBuf::from(r.text.trim());
        }
        let items: Vec<Item> = self.rows.iter().map(|r| r.item.clone()).collect();
        let copy = self.cfg.copy_mode;
        self.save();
        self.spawn("apply", "Application du plan", move |j| {
            let (jn, errs) = plan::apply(&items, copy, j);
            let mut m = format!("{} fichier(s) {}.", jn.ops.len(), if copy { "copié(s)" } else { "rangé(s)" });
            if !errs.is_empty() {
                m.push_str(&format!(" {} erreur(s) : {}", errs.len(), errs[0]));
            }
            // Le plan est consommé : on le vide via un résultat vide.
            Ok(Out::Plan(vec![], m)).map(|o| {
                let _ = jn;
                o
            })
        });
        self.undo_available = true;
        self.rows.retain(|r| !r.item.checked);
    }

    fn start_undo(&mut self) {
        let Some((p, j)) = plan::last_journal() else { return };
        self.spawn("undo", "Annulation", move |job| {
            let (n, errs) = plan::undo(&p, &j, job);
            Ok(Out::Plan(vec![], format!("{n} opération(s) annulée(s){}", if errs.is_empty() { String::new() } else { format!(", {} erreur(s) : {}", errs.len(), errs[0]) })))
        });
    }

    fn drain(&mut self) {
        for (_, res) in self.jobs.poll() {
            match res {
                Ok(m) if !m.is_empty() => self.msg = m,
                Err(e) => self.msg = format!("Erreur : {e}"),
                _ => {}
            }
        }
        let outs: Vec<Out> = self.inbox.lock().map(|mut v| std::mem::take(&mut *v)).unwrap_or_default();
        for o in outs {
            match o {
                Out::Plan(items, m) => {
                    // Un plan vide issu d'« appliquer/annuler » ne doit pas effacer un nouveau plan en cours de relecture.
                    if !items.is_empty() || self.rows.is_empty() || m.contains("annulée") {
                        self.set_plan(items);
                    }
                    self.msg = m;
                    self.undo_available = plan::last_journal().is_some();
                }
                Out::Search(r) => {
                    self.msg = format!("{} fichier(s) trouvé(s).", r.len());
                    self.results = r;
                }
                Out::Cmd(c) => self.cmd = Some(c),
                Out::Models(m) => {
                    self.msg = format!("Connexion réussie — {} modèle(s) chargé(s).", m.len());
                    if self.cfg.lm_model.is_empty() {
                        if let Some(f) = m.first() {
                            self.cfg.lm_model = f.clone();
                        }
                    }
                    self.models = m;
                }
            }
        }
    }

    // ---- Interface ------------------------------------------------------------------------

    fn busy(&self, key: &str) -> bool {
        self.jobs.running(key)
    }

    fn folders_bar(&mut self, ui: &mut egui::Ui) {
        egui::Grid::new("dirs").num_columns(3).spacing([8.0, 4.0]).show(ui, |ui| {
            ui.label("Source :");
            ui.add_sized([420.0, 22.0], egui::TextEdit::singleline(&mut self.cfg.source));
            if ui.button("Choisir…").clicked() {
                if let Some(d) = pick_dir(&self.cfg.source) {
                    if self.cfg.dest.is_empty() || self.cfg.dest == self.cfg.source {
                        self.cfg.dest = d.clone();
                    }
                    self.cfg.source = d;
                }
            }
            ui.end_row();
            ui.label("Destination :");
            ui.add_sized([420.0, 22.0], egui::TextEdit::singleline(&mut self.cfg.dest).hint_text("(identique à la source)"));
            if ui.button("Choisir…").clicked() {
                if let Some(d) = pick_dir(&self.cfg.dest) {
                    self.cfg.dest = d;
                }
            }
            ui.end_row();
        });
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.cfg.recurse, "Sous-dossiers");
            ui.separator();
            ui.radio_value(&mut self.cfg.copy_mode, false, "Déplacer");
            ui.radio_value(&mut self.cfg.copy_mode, true, "Copier");
        });
    }

    fn tab_rules(&mut self, ui: &mut egui::Ui) {
        ui.label("Règles appliquées dans l'ordre : la première qui correspond décide du dossier. Jetons : ");
        ui.label(RichText::new(rules::TOKENS).monospace().small());
        ui.checkbox(&mut self.cfg.dups, "Mettre les doublons exacts de côté dans « _Doublons » (jamais supprimés)");
        ui.add_space(4.0);
        let n = self.cfg.rules.len();
        let (mut up, mut down, mut del): (Option<usize>, Option<usize>, Option<usize>) = (None, None, None);
        egui::ScrollArea::vertical().id_salt("rules_scroll").max_height((ui.available_height() - 44.0).max(120.0)).show(ui, |ui| {
            for (i, r) in self.cfg.rules.iter_mut().enumerate() {
                let title = format!("{}{}", if r.enabled { "" } else { "⏸ " }, r.name);
                egui::CollapsingHeader::new(RichText::new(format!("{}. {title}  →  {}", i + 1, r.template)).strong()).id_salt(("rule", i)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut r.enabled, "Active");
                        ui.add(egui::TextEdit::singleline(&mut r.name).desired_width(200.0));
                        if ui.add_enabled(i > 0, egui::Button::new("▲")).clicked() { up = Some(i); }
                        if ui.add_enabled(i + 1 < n, egui::Button::new("▼")).clicked() { down = Some(i); }
                        if ui.button("Supprimer").clicked() { del = Some(i); }
                    });
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Types :");
                        for k in Kind::ALL {
                            let mut on = r.kinds.contains(&k);
                            if ui.checkbox(&mut on, k.label()).changed() {
                                r.kinds.retain(|x| *x != k);
                                if on { r.kinds.push(k); }
                            }
                        }
                    });
                    egui::Grid::new(("rg", i)).num_columns(2).show(ui, |ui| {
                        ui.label("Extensions :");
                        ui.add(egui::TextEdit::singleline(&mut r.exts).hint_text("jpg, png (vide = toutes)").desired_width(260.0));
                        ui.end_row();
                        ui.label("Nom (regex) :");
                        ui.add(egui::TextEdit::singleline(&mut r.name_regex).hint_text("(?i)screenshot").desired_width(260.0));
                        ui.end_row();
                        ui.label("Taille (Mo) :");
                        ui.horizontal(|ui| {
                            ui.add(egui::DragValue::new(&mut r.min_mb).range(0.0..=1e6).prefix("min "));
                            ui.add(egui::DragValue::new(&mut r.max_mb).range(0.0..=1e6).prefix("max "));
                        });
                        ui.end_row();
                        ui.label("Date connue :");
                        egui::ComboBox::from_id_salt(("nd", i)).selected_text(["indifférent", "oui (EXIF / nom)", "non"][r.need_date as usize % 3]).show_ui(ui, |ui| {
                            for (v, l) in ["indifférent", "oui (EXIF / nom)", "non"].iter().enumerate() {
                                ui.selectable_value(&mut r.need_date, v as u8, *l);
                            }
                        });
                        ui.end_row();
                        ui.label("Dossier :");
                        ui.add(egui::TextEdit::singleline(&mut r.template).desired_width(320.0));
                        ui.end_row();
                    });
                });
            }
        });
        if let Some(i) = up { self.cfg.rules.swap(i, i - 1); }
        if let Some(i) = down { self.cfg.rules.swap(i, i + 1); }
        if let Some(i) = del { self.cfg.rules.remove(i); }
        ui.horizontal(|ui| {
            if ui.button("+ Règle").clicked() { self.cfg.rules.push(Rule::default()); }
            if ui.button("Règles par défaut").clicked() { self.cfg.rules = rules::default_rules(); }
            ui.separator();
            if ui.add_enabled(!self.busy("rules"), egui::Button::new(RichText::new("▶ Analyser et proposer un plan").strong())).clicked() {
                self.start_rules();
            }
        });
    }

    fn tab_ai(&mut self, ui: &mut egui::Ui) {
        ui.label("L'IA (LM Studio, 100 % local) range les photos et textes dans VOS catégories. Les photos sont réduites avant envoi.");
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.ai_photos, "Photos (vision)");
            ui.checkbox(&mut self.ai_text, "Fichiers texte (txt, md, csv, html)");
            ui.checkbox(&mut self.ai_only_other, "Seulement ceux que les règles ne classent pas");
        });
        ui.horizontal(|ui| {
            ui.label("Dossier :");
            ui.add(egui::TextEdit::singleline(&mut self.cfg.ai_template).desired_width(300.0));
            ui.label("Décocher si confiance <");
            ui.add(egui::DragValue::new(&mut self.ai_min_conf).range(0.0..=1.0).speed(0.01));
        });
        ui.label(RichText::new("Jetons : {categorie} {titre} {année} {mois} {type} {nom} …").small().weak());
        ui.separator();
        ui.label(RichText::new("Catégories (l'IA doit choisir dans cette liste)").strong());
        let mut del = None;
        egui::ScrollArea::vertical().id_salt("cats").max_height((ui.available_height() - 50.0).max(80.0)).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                for (i, c) in self.cfg.categories.iter_mut().enumerate() {
                    ui.add(egui::TextEdit::singleline(c).desired_width(130.0));
                    if ui.small_button("✕").clicked() { del = Some(i); }
                }
            });
        });
        if let Some(i) = del { self.cfg.categories.remove(i); }
        ui.horizontal(|ui| {
            if ui.button("+ Catégorie").clicked() { self.cfg.categories.push("Nouvelle".into()); }
            if ui.add_enabled(!self.busy("ai"), egui::Button::new(RichText::new("▶ Classer avec l'IA").strong())).clicked() {
                self.start_ai();
            }
        });
    }

    fn tab_command(&mut self, ui: &mut egui::Ui) {
        ui.label("Décrivez ce que vous voulez en français ; l'IA le traduit en filtre + action, puis vous validez.");
        ui.label(RichText::new("Ex. : « retrouve les vidéos de plus de 500 Mo de 2022 » · « renomme les photos avec la date en préfixe » · « range les PDF par année »").small().weak());
        ui.add(egui::TextEdit::multiline(&mut self.cmd_text).desired_rows(2).desired_width(f32::INFINITY));
        ui.horizontal(|ui| {
            if ui.add_enabled(!self.busy("cmd_parse") && !self.cmd_text.trim().is_empty(), egui::Button::new("Interpréter")).clicked() {
                self.cmd = None;
                self.start_cmd_parse();
            }
            if let Some(c) = &self.cmd {
                let label = match c.action {
                    cmd::Action::Search => "▶ Rechercher",
                    _ => "▶ Construire le plan",
                };
                if ui.add_enabled(!self.busy("cmd_run"), egui::Button::new(RichText::new(label).strong())).clicked() {
                    self.start_cmd_run();
                }
            }
        });
        if let Some(c) = &self.cmd {
            ui.group(|ui| {
                let a = match &c.action {
                    cmd::Action::Search => "Recherche".to_string(),
                    cmd::Action::Rename(o) => format!("Renommage ({} opération(s))", o.len()),
                    cmd::Action::Sort(t) => format!("Rangement vers « {t} »"),
                };
                ui.label(RichText::new(a).strong());
                ui.label(format!("Filtre : {}", c.filter.describe()));
                if !c.explain.is_empty() {
                    ui.label(RichText::new(&c.explain).italics());
                }
            });
        }
        if !self.results.is_empty() {
            ui.separator();
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{} résultat(s)", self.results.len())).strong());
                if ui.button("Ouvrir le dossier source dans FileForge").clicked() {
                    launch("fileforge", &[self.src()]);
                }
            });
            let rows = self.results.len();
            egui::ScrollArea::vertical().id_salt("res").show_rows(ui, 18.0, rows, |ui, range| {
                for f in &self.results[range] {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("{:>9}", human(f.size))).monospace().weak());
                        if ui.link(f.path.display().to_string()).on_hover_text("Ouvrir le dossier").clicked() {
                            if let Some(d) = f.path.parent() { launch("fileforge", &[d.to_path_buf()]); }
                        }
                    });
                }
            });
        }
    }

    fn tab_ident(&mut self, ui: &mut egui::Ui) {
        ui.label("Retrouve les vrais noms : films (TMDB / iTunes / Wikipédia), musique (MusicBrainz), ROMs (fichiers DAT No-Intro / TOSEC / Redump par CRC32).");
        ui.label(RichText::new("Les ROMs sont reconnues par empreinte : chargez vos DAT dans Réglages. Les noms proposés sont à valider dans le plan (fiabilité < 75 % : décoché).").small().weak());
        ui.checkbox(&mut self.cfg.llm_title, "Laisser l'IA extraire le titre depuis un nom de fichier brouillon (si LM Studio répond)");
        ui.horizontal(|ui| {
            ui.label("Films :");
            ui.add(egui::TextEdit::singleline(&mut self.cfg.film_template).desired_width(200.0));
            ui.label("Musique :");
            ui.add(egui::TextEdit::singleline(&mut self.cfg.music_template).desired_width(200.0));
        });
        ui.label(RichText::new("Jetons : {titre} {année} {artiste} {album}").small().weak());
        if ui.add_enabled(!self.busy("ident"), egui::Button::new(RichText::new("▶ Identifier les vidéos, musiques et ROMs").strong())).clicked() {
            self.start_ident();
        }
    }

    fn tab_settings(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().id_salt("set").show(ui, |ui| {
            ui.label(RichText::new("IA locale — LM Studio").strong());
            egui::Grid::new("lm").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
                ui.label("URL du serveur :");
                ui.add(egui::TextEdit::singleline(&mut self.cfg.lm_url).desired_width(300.0));
                ui.end_row();
                ui.label("Modèle :");
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.cfg.lm_model).desired_width(220.0).hint_text("(vide = modèle chargé)"));
                    if !self.models.is_empty() {
                        egui::ComboBox::from_id_salt("models").selected_text("▼").show_ui(ui, |ui| {
                            for m in self.models.clone() {
                                ui.selectable_value(&mut self.cfg.lm_model, m.clone(), m);
                            }
                        });
                    }
                });
                ui.end_row();
                ui.label("Clé API :");
                ui.add(egui::TextEdit::singleline(&mut self.cfg.lm_key).password(true).desired_width(300.0).hint_text("(inutile pour LM Studio)"));
                ui.end_row();
                ui.label("Délai (s) :");
                ui.add(egui::DragValue::new(&mut self.cfg.lm_timeout).range(10..=1200));
                ui.end_row();
                ui.label("Miniatures (px) :");
                ui.add(egui::DragValue::new(&mut self.cfg.thumb_px).range(128..=2048).speed(16));
                ui.end_row();
            });
            ui.checkbox(&mut self.cfg.no_think, "Mode rapide Qwen3 (/no_think : pas de raisonnement)");
            ui.horizontal(|ui| {
                if ui.add_enabled(!self.busy("test"), egui::Button::new("Tester la connexion")).clicked() {
                    self.save();
                    let c = self.cfg.clone();
                    self.spawn("test", "Test LM Studio", move |_| Ok(Out::Models(llm::Llm::new(&c).models()?)));
                }
                if ui.add_enabled(!self.busy("test2") && !self.busy("test"), egui::Button::new("Test de réponse")).clicked() {
                    self.save();
                    let c = self.cfg.clone();
                    self.spawn("test2", "Test de réponse", move |_| {
                        let r = llm::Llm::new(&c).chat("Réponds en une courte phrase.", "Dis bonjour en français.", &[], 60)?;
                        Ok(Out::Plan(vec![], format!("Réponse du modèle : {r}")))
                    });
                }
            });
            ui.separator();
            ui.label(RichText::new("Identification").strong());
            egui::Grid::new("web").num_columns(2).show(ui, |ui| {
                ui.label("Clé TMDB (facultative) :");
                ui.add(egui::TextEdit::singleline(&mut self.cfg.tmdb_key).password(true).desired_width(300.0).hint_text("sans clé : iTunes + Wikipédia"));
                ui.end_row();
                ui.label("Langue :");
                egui::ComboBox::from_id_salt("wl").selected_text(&self.cfg.web_lang).show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.cfg.web_lang, "fr".into(), "fr");
                    ui.selectable_value(&mut self.cfg.web_lang, "en".into(), "en");
                });
                ui.end_row();
            });
            ui.label("Fichiers DAT (ROMs) :");
            let mut del = None;
            for (i, p) in self.cfg.dat_files.iter().enumerate() {
                ui.horizontal(|ui| {
                    if ui.small_button("✕").clicked() { del = Some(i); }
                    ui.label(p.display().to_string());
                });
            }
            if let Some(i) = del { self.cfg.dat_files.remove(i); }
            if ui.button("Ajouter un DAT…").clicked() {
                if let Some(f) = rfd::FileDialog::new().add_filter("DAT", &["dat", "xml"]).pick_files() {
                    self.cfg.dat_files.extend(f);
                }
            }
            ui.separator();
            ui.label(RichText::new("Apparence (partagée avec MediaForge)").strong());
            let mut id = self.sett.theme.clone();
            if ff_kit::theme::style_picker(ui, &mut id, self.sett.dark, false) {
                self.sett.theme = id;
                self.save_theme(ui.ctx());
            }
            if ui.checkbox(&mut self.sett.dark, "Mode sombre").changed() {
                self.save_theme(ui.ctx());
            }
            ui.separator();
            if ui.button("Enregistrer").clicked() {
                self.save();
                self.msg = "Réglages enregistrés.".into();
            }
        });
    }

    fn save_theme(&mut self, ctx: &egui::Context) {
        let mut s = ff_profiles::load();
        s.theme = self.sett.theme.clone();
        s.dark = self.sett.dark;
        let _ = ff_profiles::save(&s);
        ff_kit::theme::apply(ctx, ff_kit::theme::Style::from_id(&self.sett.theme), self.sett.dark);
    }

    fn plan_panel(&mut self, ui: &mut egui::Ui) {
        let pal = ff_kit::theme::current();
        let checked = self.rows.iter().filter(|r| r.item.checked).count();
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("Plan : {checked}/{} cochés", self.rows.len())).strong());
            ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text("filtrer…").desired_width(160.0));
            if ui.button("Tout").clicked() { self.rows.iter_mut().for_each(|r| r.item.checked = true); }
            if ui.button("Aucun").clicked() { self.rows.iter_mut().for_each(|r| r.item.checked = false); }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add_enabled(checked > 0 && !self.busy("apply"), egui::Button::new(RichText::new(if self.cfg.copy_mode { "✔ Copier les fichiers cochés" } else { "✔ Appliquer (déplacer)" }).strong().color(Color32::WHITE)).fill(pal.ok)).clicked() {
                    self.start_apply();
                }
                if ui.add_enabled(self.undo_available && !self.busy("undo"), egui::Button::new("↶ Annuler la dernière opération")).clicked() {
                    self.start_undo();
                }
            });
        });
        let f = self.filter.to_lowercase();
        let idx: Vec<usize> = (0..self.rows.len()).filter(|&i| f.is_empty() || self.rows[i].item.src.display().to_string().to_lowercase().contains(&f) || self.rows[i].text.to_lowercase().contains(&f)).collect();
        let base = self.src();
        egui::ScrollArea::both().id_salt("plan").auto_shrink([false, false]).show_rows(ui, 24.0, idx.len(), |ui, range| {
            for k in range {
                let i = idx[k];
                let r = &mut self.rows[i];
                ui.horizontal(|ui| {
                    ui.checkbox(&mut r.item.checked, "");
                    let rel = r.item.src.strip_prefix(&base).unwrap_or(&r.item.src).display().to_string();
                    ui.add_sized([230.0, 20.0], egui::Label::new(rel).truncate());
                    ui.label("→");
                    ui.add(egui::TextEdit::singleline(&mut r.text).desired_width(360.0));
                    let c = match r.item.origin { Origin::Dup => pal.err, Origin::Ai => pal.accent, _ => pal.ok };
                    ui.label(RichText::new(r.item.origin.label()).color(c).small());
                    ui.label(RichText::new(&r.item.reason).small().weak());
                });
            }
        });
    }
}

fn human(b: u64) -> String {
    let (mut v, mut u) = (b as f64, 0);
    while v >= 1024.0 && u < 4 {
        v /= 1024.0;
        u += 1;
    }
    format!("{:.1} {}", v, ["o", "Ko", "Mo", "Go", "To"][u])
}

fn launch(name: &str, args: &[PathBuf]) {
    let f = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    let exe = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join(&f))).filter(|p| p.exists()).unwrap_or_else(|| PathBuf::from(f));
    let _ = std::process::Command::new(exe).args(args).spawn();
}

impl eframe::App for App {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = root.ctx().clone();
        if self.theme_check.elapsed().as_secs() >= 2 {
            self.theme_check = std::time::Instant::now();
            let sh = ff_profiles::load();
            if sh.theme != self.sett.theme || sh.dark != self.sett.dark {
                self.sett.theme = sh.theme;
                self.sett.dark = sh.dark;
                ff_kit::theme::apply(&ctx, ff_kit::theme::Style::from_id(&self.sett.theme), self.sett.dark);
            }
            ctx.request_repaint_after(std::time::Duration::from_secs(2));
        }
        self.drain();
        if self.jobs.any() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        let pal = ff_kit::theme::current();
        let hdr = pal.header.unwrap_or(pal.panel);
        let on = ff_kit::theme::on_color(hdr);

        if root.ctx().input(|i| i.key_pressed(egui::Key::F1)) {
            let a = match self.tab { Tab::Rules => "tri-regles", Tab::Ai => "ia", Tab::Command => "commandes", Tab::Ident => "identification", Tab::Settings => "reglages" };
            ff_kit::help::trigger(root.ctx(), "autoforge", a);
        }
        ff_kit::help::show_error(root.ctx());
        egui::Panel::top("hdr").frame(egui::Frame::new().fill(hdr).inner_margin(egui::Margin::symmetric(10, 8))).show(root, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("AutoForge").size(20.0).strong().color(on));
                ui.add_space(16.0);
                for (t, l) in [(Tab::Rules, "Trier (règles)"), (Tab::Ai, "IA : classer"), (Tab::Command, "Commande"), (Tab::Ident, "Identifier"), (Tab::Settings, "Réglages")] {
                    if ui.add(egui::Button::selectable(self.tab == t, RichText::new(l).color(if self.tab == t { pal.text } else { on }))).clicked() {
                        self.tab = t;
                    }
                }
                let anchor = match self.tab { Tab::Rules => "tri-regles", Tab::Ai => "ia", Tab::Command => "commandes", Tab::Ident => "identification", Tab::Settings => "reglages" };
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ff_kit::help::button(ui, "autoforge", anchor, "Aide (F1)");
                });
            });
        });

        egui::Panel::bottom("footer").show(root, |ui| {
            for job in self.jobs.active() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(format!("{} — {}", job.label, job.detail()));
                    match job.fraction() {
                        Some(f) => { ui.add(egui::ProgressBar::new(f).desired_width(200.0).show_percentage()); }
                        None => { ui.add(egui::ProgressBar::new(0.0).desired_width(200.0).animate(true)); }
                    }
                    if ui.small_button("Annuler").clicked() { job.cancel(); }
                });
            }
            ui.horizontal(|ui| {
                ui.label(RichText::new(&self.msg).color(pal.weak));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(format!("AutoForge v{VERSION}")).small().weak());
                });
            });
        });

        egui::Panel::bottom("plan").resizable(true).default_size(260.0).min_size(120.0).show(root, |ui| {
            self.plan_panel(ui);
        });

        egui::CentralPanel::default().show(root, |ui| {
            self.folders_bar(ui);
            ui.separator();
            match self.tab {
                Tab::Rules => self.tab_rules(ui),
                Tab::Ai => self.tab_ai(ui),
                Tab::Command => self.tab_command(ui),
                Tab::Ident => self.tab_ident(ui),
                Tab::Settings => self.tab_settings(ui),
            }
        });
    }
}

fn main() -> eframe::Result<()> {
    let start = std::env::args_os().nth(1).map(PathBuf::from).filter(|p| Path::new(p).is_dir());
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title("AutoForge").with_inner_size([1100.0, 760.0]).with_min_inner_size([760.0, 520.0]),
        ..Default::default()
    };
    eframe::run_native("AutoForge", options, Box::new(move |cc| Ok(Box::new(App::new(cc, start)))))
}
