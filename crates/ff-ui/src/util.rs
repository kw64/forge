//! Utilitaires de l'interface : ouverture de dossiers, formatage, extensions média. v0.3.0
use std::path::{Path, PathBuf};
use std::process::Command;

/// Extensions reconnues lors d'un dépôt de dossier ou d'un filtre de dialogue.
pub const MEDIA_EXT: &[&str] = &[
    "mp4", "mkv", "avi", "mov", "wmv", "flv", "webm", "mpg", "mpeg", "m4v", "3gp", "ts", "vob", "gif",
    "mp3", "m4a", "aac", "ogg", "opus", "flac", "wav", "wma", "ape", "aiff", "mka",
    "jpg", "jpeg", "png", "webp", "bmp", "tif", "tiff", "avif",
];

pub fn is_media(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| MEDIA_EXT.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// Développe un chemin déposé : un fichier tel quel, un dossier -> ses fichiers média (un niveau).
pub fn expand_dropped(p: PathBuf) -> Vec<PathBuf> {
    if p.is_dir() {
        let mut v: Vec<PathBuf> = std::fs::read_dir(&p)
            .map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.path()).filter(|q| q.is_file() && is_media(q)).collect())
            .unwrap_or_default();
        v.sort();
        v
    } else {
        vec![p]
    }
}

/// Ouvre le dossier contenant le fichier (sélectionné sous Windows), sans passer par un shell.
pub fn reveal(path: &Path) {
    let dir = path.parent().unwrap_or(path);
    #[cfg(windows)]
    {
        let _ = Command::new("explorer").arg("/select,").arg(path).spawn();
        let _ = dir;
    }
    #[cfg(target_os = "macos")]
    {
        let _ = Command::new("open").arg(dir).spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = Command::new("xdg-open").arg(dir).spawn();
    }
}

/// « 3,2 x » / « 1:05 » : formatage lisible.
pub fn fmt_secs(s: f64) -> String {
    let t = s.max(0.0) as u64;
    if t >= 3600 {
        format!("{}:{:02}:{:02}", t / 3600, (t / 60) % 60, t % 60)
    } else {
        format!("{}:{:02}", t / 60, t % 60)
    }
}
