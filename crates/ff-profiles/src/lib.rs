//! ff-profiles : réglages et profils utilisateur (JSON), mode portable, journal. v0.3.0
use anyhow::{Context, Result};
use ff_core::{builtin_profiles, HwAccel, Lang, Profile};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::PathBuf;

/// Réglages persistants de l'application.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub output_dir: Option<PathBuf>,
    pub ffmpeg_dir: Option<PathBuf>,
    /// FFmpeg « full » (GPU) : dossier choisi à la main (sinon dossier de données).
    pub ffmpeg_full_dir: Option<PathBuf>,
    /// Version utilisée par défaut : « lite » (celle détectée) ou « full ».
    pub ffmpeg_flavor: String,
    /// Proposer la version full quand un encodage GPU est demandé avec une version lite.
    pub ffmpeg_ask_full: bool,
    pub parallel: usize,
    pub dark: bool,
    /// Style visuel : colorful, sober ou office (combiné à `dark`).
    pub theme: String,
    /// Affichage de la liste en grandes icônes (cartes) plutôt qu'en lignes.
    pub icon_view: bool,
    /// FileForge : un simple clic ouvre dossiers et archives (Ctrl+clic = sélection).
    pub single_click: bool,
    /// Vidéos en ligne : dossier de téléchargement (défaut : Téléchargements).
    pub online_dir: Option<std::path::PathBuf>,
    /// FileForge : identifiants des dossiers personnels masqués dans « Lecteurs ».
    pub hidden_places: Vec<String>,
    /// FileForge : dossiers ajoutés à « Lecteurs ».
    pub custom_places: Vec<String>,
    /// FileForge : identifiants des boutons de la barre centrale masqués.
    pub hidden_buttons: Vec<String>,
    /// Fenêtres de réglages : un clic en dehors les ferme.
    pub close_on_outside: bool,
    pub lang: Lang,
    pub hw: HwAccel,
    pub last_profile: String,
    pub custom_profiles: Vec<Profile>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            output_dir: None,
            ffmpeg_dir: None,
            ffmpeg_full_dir: None,
            ffmpeg_flavor: "lite".into(),
            ffmpeg_ask_full: true,
            parallel: 2,
            dark: true,
            theme: "colorful".into(),
            icon_view: false,
            single_click: false,
            online_dir: None,
            hidden_places: vec![],
            custom_places: vec![],
            hidden_buttons: vec![],
            close_on_outside: true,
            lang: Lang::Fr,
            hw: HwAccel::Auto,
            last_profile: "mp4".into(),
            custom_profiles: Vec::new(),
        }
    }
}

impl Settings {
    /// Profils intégrés suivis des profils utilisateur.
    pub fn all_profiles(&self) -> Vec<Profile> {
        let mut v = builtin_profiles();
        v.extend(self.custom_profiles.iter().cloned().map(|mut p| {
            p.builtin = false;
            p
        }));
        v
    }
}

/// Dossier de données : à côté de l'exécutable si `mediaforge.portable` existe, sinon dossier de config de l'OS.
pub fn data_dir() -> PathBuf {
    if let Ok(me) = std::env::current_exe() {
        if let Some(dir) = me.parent() {
            if dir.join("mediaforge.portable").exists() {
                return dir.join("data");
            }
        }
    }
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("MediaForge")
}

fn settings_path() -> PathBuf {
    data_dir().join("settings.json")
}

/// Charge les réglages (valeurs par défaut si absents ou illisibles).
pub fn load() -> Settings {
    std::fs::read_to_string(settings_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Enregistre les réglages (écriture atomique : fichier temporaire puis renommage).
pub fn save(s: &Settings) -> Result<()> {
    let dir = data_dir();
    std::fs::create_dir_all(&dir).with_context(|| format!("création de {}", dir.display()))?;
    let tmp = dir.join("settings.json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(s)?)?;
    std::fs::rename(&tmp, settings_path())?;
    Ok(())
}

/// Chemin du journal d'erreurs.
pub fn log_path() -> PathBuf {
    data_dir().join("mediaforge.log")
}

/// Ajoute une ligne au journal (best effort ; le journal est tronqué au-delà de 1 Mo).
pub fn log(msg: &str) {
    let path = log_path();
    let _ = std::fs::create_dir_all(data_dir());
    if std::fs::metadata(&path).map(|m| m.len() > 1_000_000).unwrap_or(false) {
        let _ = std::fs::remove_file(&path);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{msg}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_and_roundtrip() {
        let mut s = Settings::default();
        s.custom_profiles.push(Profile { id: "x".into(), label: "X".into(), extension: "mp4".into(), args: vec!["-c".into(), "copy".into()], builtin: true, spec: None });
        let json = serde_json::to_string(&s).unwrap();
        let back: Settings = serde_json::from_str(&json).unwrap();
        assert!(back.all_profiles().last().map(|p| !p.builtin).unwrap());
        // un JSON partiel reste lisible
        let partial: Settings = serde_json::from_str(r#"{"parallel":3}"#).unwrap();
        assert_eq!(partial.parallel, 3);
    }
}
