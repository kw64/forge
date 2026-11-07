//! Réglages du lecteur et positions de reprise (JSON dans le dossier de données de MediaForge). v0.4.0
use crate::playlist::Repeat;
use ff_core::Lang;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PlayerSettings {
    pub volume: f64,
    pub muted: bool,
    pub shuffle: bool,
    pub repeat: Repeat,
    pub slideshow_secs: u32,
    pub load_siblings: bool,
    pub hwdec: bool,
    pub hq_scaling: bool,
    /// Mode cinéma : bords de l'écran remplis par les côtés de la vidéo, en miroir et flous.
    pub cinema: bool,
    /// Flou des bandes du mode cinéma : 0 (aucun) à 6.
    pub cinema_blur: u32,
    /// Derushage : timecodes en surimpression (SMPTE, drop-frame, images, ms, reste) et notes horodatées.
    pub tc_on: bool,
    pub tc_smpte: bool,
    pub tc_drop: bool,
    pub tc_frames: bool,
    pub tc_ms: bool,
    pub tc_remain: bool,
    /// Coin : 0 haut-gauche, 1 haut-droite, 2 bas-gauche, 3 bas-droite.
    pub tc_corner: u8,
    pub tc_size: f32,
    pub notes_show: bool,
    pub notes_dur: f32,
    pub resume: bool,
    /// Résidence : la fermeture réduit la fenêtre au lieu de quitter (réaffichage instantané).
    pub keep_resident: bool,
    /// Format copié par la pipette : 0 web (#RRGGBB), 1 rgb(r, g, b), 2 les deux.
    pub pick_fmt: u8,
    /// Lancé avec Windows (réduit).
    pub autostart: bool,
    pub lang: Lang,
    pub mpv_dir: Option<PathBuf>,
}

impl Default for PlayerSettings {
    fn default() -> Self {
        Self {
            volume: 80.0,
            muted: false,
            shuffle: false,
            repeat: Repeat::Off,
            slideshow_secs: 4,
            load_siblings: true,
            hwdec: true,
            hq_scaling: true,
            cinema: false,
            cinema_blur: 4,
            tc_on: false,
            tc_smpte: true,
            tc_drop: false,
            tc_frames: false,
            tc_ms: false,
            tc_remain: false,
            tc_corner: 0,
            tc_size: 22.0,
            notes_show: true,
            notes_dur: 5.0,
            resume: true,
            keep_resident: false,
            pick_fmt: 0,
            autostart: false,
            lang: ff_profiles::load().lang,
            mpv_dir: None,
        }
    }
}

fn dir() -> PathBuf {
    ff_profiles::data_dir()
}

pub fn load_settings() -> PlayerSettings {
    std::fs::read_to_string(dir().join("player.json")).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub fn save_settings(s: &PlayerSettings) {
    let _ = std::fs::create_dir_all(dir());
    if let Ok(j) = serde_json::to_vec_pretty(s) {
        let tmp = dir().join("player.json.tmp");
        if std::fs::write(&tmp, j).is_ok() {
            let _ = std::fs::rename(&tmp, dir().join("player.json"));
        }
    }
}

/// Positions de reprise (clé = chemin du fichier), limitées aux 200 plus récentes.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Resume {
    map: HashMap<String, f64>,
    order: Vec<String>,
}

impl Resume {
    pub fn load() -> Self {
        std::fs::read_to_string(dir().join("resume.json")).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }
    pub fn save(&self) {
        let _ = std::fs::create_dir_all(dir());
        if let Ok(j) = serde_json::to_vec(self) {
            let _ = std::fs::write(dir().join("resume.json"), j);
        }
    }
    pub fn get(&self, p: &std::path::Path) -> Option<f64> {
        self.map.get(&p.to_string_lossy().into_owned()).copied()
    }
    /// Mémorise une position ; l'efface si le média est (presque) terminé ou à peine commencé.
    pub fn record(&mut self, p: &std::path::Path, pos: f64, dur: f64) {
        let key = p.to_string_lossy().into_owned();
        self.order.retain(|k| k != &key);
        if dur >= 300.0 && pos > 10.0 && pos < dur - 15.0 {
            self.map.insert(key.clone(), pos);
            self.order.push(key);
            while self.order.len() > 200 {
                let old = self.order.remove(0);
                self.map.remove(&old);
            }
        } else {
            self.map.remove(&key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resume_rules() {
        let mut r = Resume::default();
        let p = std::path::Path::new("/v/film.mkv");
        r.record(p, 600.0, 3600.0);
        assert_eq!(r.get(p), Some(600.0));
        r.record(p, 3590.0, 3600.0); // quasi terminé
        assert_eq!(r.get(p), None);
        r.record(p, 5.0, 3600.0); // à peine commencé
        assert_eq!(r.get(p), None);
        r.record(p, 100.0, 120.0); // média court : jamais mémorisé
        assert_eq!(r.get(p), None);
    }
}
