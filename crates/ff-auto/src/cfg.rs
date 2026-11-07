//! Configuration d'AutoForge (autoforge.json dans le dossier de données partagé). v0.1.0
use crate::rules::{default_rules, Rule};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Serveur compatible OpenAI (LM Studio : http://localhost:1234/v1).
    pub lm_url: String,
    pub lm_model: String,
    pub lm_key: String,
    pub lm_timeout: u64,
    /// Ajoute /no_think (Qwen3) pour des réponses plus rapides.
    pub no_think: bool,
    /// Côté max (px) des miniatures envoyées à l'IA.
    pub thumb_px: u32,
    pub categories: Vec<String>,
    pub rules: Vec<Rule>,
    pub source: String,
    pub dest: String,
    pub recurse: bool,
    pub copy_mode: bool,
    pub dups: bool,
    /// Gabarit AI : chemin relatif où classer un fichier selon sa catégorie.
    pub ai_template: String,
    pub dat_files: Vec<PathBuf>,
    pub tmdb_key: String,
    pub film_template: String,
    pub music_template: String,
    pub web_lang: String,
    pub llm_title: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            lm_url: "http://localhost:1234/v1".into(),
            lm_model: String::new(),
            lm_key: String::new(),
            lm_timeout: 180,
            no_think: true,
            thumb_px: 768,
            categories: ["Famille", "Vacances", "Nature", "Animaux", "Nourriture", "Documents", "Captures d'écran", "Travail", "Autre"].map(String::from).to_vec(),
            rules: default_rules(),
            source: String::new(),
            dest: String::new(),
            recurse: true,
            copy_mode: false,
            dups: true,
            ai_template: "{type}/{categorie}/{année}".into(),
            dat_files: vec![],
            tmdb_key: String::new(),
            film_template: "{titre} ({année})".into(),
            music_template: "{artiste} - {titre}".into(),
            web_lang: "fr".into(),
            llm_title: true,
        }
    }
}

fn path() -> PathBuf {
    ff_profiles::data_dir().join("autoforge.json")
}

pub fn load() -> Config {
    std::fs::read_to_string(path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub fn save(c: &Config) -> Result<(), String> {
    let _ = std::fs::create_dir_all(ff_profiles::data_dir());
    let tmp = path().with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(c).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path()).map_err(|e| e.to_string())
}
