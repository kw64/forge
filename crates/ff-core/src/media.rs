//! Classification des médias par extension, listes de dossier et tri naturel. v0.4.0
use std::cmp::Ordering;
use std::path::{Path, PathBuf};

use crate::Profile;

pub const VIDEO_EXT: &[&str] = &[
    "mp4", "mkv", "avi", "mov", "wmv", "flv", "webm", "mpg", "mpeg", "m4v", "3gp", "ts", "m2ts", "mts", "vob", "ogv", "divx", "asf", "rm", "rmvb",
];
pub const AUDIO_EXT: &[&str] = &[
    "mp3", "m4a", "aac", "ogg", "oga", "opus", "flac", "wav", "wma", "ape", "aiff", "aif", "mka", "wv", "ac3", "dts", "m4b", "mpc",
    // Musiques rétro : trackers Amiga/PC (FFmpeg + libopenmpt), Atari ST (ym, sndh), Commodore 64 (sid, rendu par ff-chip)
    "mod", "xm", "s3m", "it", "mtm", "669", "stm", "okt", "med", "far", "ult", "amf", "dbm", "dsm", "gdm", "imf", "j2b", "mdl", "ptm", "psm", "umx", "mo3", "ym", "sndh", "sid",
];
pub const IMAGE_EXT: &[&str] = &["jpg", "jpeg", "jfif", "png", "webp", "bmp", "tif", "tiff", "avif", "gif", "jxl", "ico", "tga"];

/// Documents / graphiques rendus en image par le lecteur (PDF, Illustrator, Photoshop, EPS).
pub const DOC_EXT: &[&str] = &["pdf", "ai", "psd", "eps"];

pub fn is_doc(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).map(|e| DOC_EXT.contains(&e.to_ascii_lowercase().as_str())).unwrap_or(false)
}

/// Maillages 3D affichés par le lecteur.
pub const MESH_EXT: &[&str] = &["stl", "obj"];

pub fn is_mesh(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).map(|e| MESH_EXT.contains(&e.to_ascii_lowercase().as_str())).unwrap_or(false)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Video,
    Audio,
    Image,
}

/// Nature d'un fichier d'après son extension (insensible à la casse).
pub fn kind_of(p: &Path) -> Option<MediaKind> {
    let e = p.extension()?.to_str()?.to_ascii_lowercase();
    if VIDEO_EXT.contains(&e.as_str()) {
        Some(MediaKind::Video)
    } else if AUDIO_EXT.contains(&e.as_str()) {
        Some(MediaKind::Audio)
    } else if IMAGE_EXT.contains(&e.as_str()) {
        Some(MediaKind::Image)
    } else {
        None
    }
}

/// Tri « naturel » insensible à la casse : « img2 » avant « img10 ».
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut ai, mut bi) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, _) => return Ordering::Less,
            (_, None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let take = |it: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut s = String::new();
                    while let Some(c) = it.peek().copied().filter(|c| c.is_ascii_digit()) {
                        s.push(c);
                        it.next();
                    }
                    s.trim_start_matches('0').to_string()
                };
                let (na, nb) = (take(&mut ai), take(&mut bi));
                let o = na.len().cmp(&nb.len()).then_with(|| na.cmp(&nb));
                if o != Ordering::Equal {
                    return o;
                }
            }
            (Some(x), Some(y)) => {
                let o = x.to_lowercase().cmp(y.to_lowercase());
                if o != Ordering::Equal {
                    return o;
                }
                ai.next();
                bi.next();
            }
        }
    }
}

/// Fichiers « de même famille » dans le dossier d'un fichier (audio et vidéo ensemble, images à part), triés naturellement.
pub fn siblings(p: &Path) -> Vec<PathBuf> {
    let Some(kind) = kind_of(p) else { return vec![p.to_path_buf()] };
    let group = |k: MediaKind| k == MediaKind::Image;
    let Some(dir) = p.parent().filter(|d| !d.as_os_str().is_empty()) else { return vec![p.to_path_buf()] };
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|q| q.is_file() && kind_of(q).map(|k| group(k) == group(kind)).unwrap_or(false))
                .collect()
        })
        .unwrap_or_default();
    v.sort_by(|a, b| natural_cmp(&a.file_name().unwrap_or_default().to_string_lossy(), &b.file_name().unwrap_or_default().to_string_lossy()));
    if !v.contains(&p.to_path_buf()) {
        v.push(p.to_path_buf());
    }
    v
}

#[cfg(test)]
mod tests {
    #[test]
    fn profiles_by_input_kind() {
        let ps = crate::builtin_profiles();
        let ok = |id: &str, k| profile_accepts(ps.iter().find(|p| p.id == id).unwrap(), k);
        assert!(ok("mp4", MediaKind::Video) && ok("mp3", MediaKind::Video) && ok("gif", MediaKind::Video));
        assert!(!ok("mp4", MediaKind::Audio) && ok("flac", MediaKind::Audio) && !ok("jpg", MediaKind::Audio));
        assert!(ok("png", MediaKind::Image) && !ok("mp3", MediaKind::Image) && !ok("gif", MediaKind::Image));
    }

    use super::*;
    #[test]
    fn kinds() {
        assert_eq!(kind_of(Path::new("a/B.MKV")), Some(MediaKind::Video));
        assert_eq!(kind_of(Path::new("x.FLAC")), Some(MediaKind::Audio));
        assert_eq!(kind_of(Path::new("p.JPeG")), Some(MediaKind::Image));
        assert_eq!(kind_of(Path::new("notes.txt")), None);
        assert_eq!(kind_of(Path::new("noext")), None);
    }
    #[test]
    fn natural() {
        let mut v = vec!["img10.jpg", "IMG2.jpg", "img1.jpg", "img02.jpg"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, vec!["img1.jpg", "IMG2.jpg", "img02.jpg", "img10.jpg"]);
    }
    #[test]
    fn siblings_sorted_and_grouped() {
        let d = std::env::temp_dir().join(format!("mf_sib_{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        for n in ["b10.mp4", "b2.mp4", "c.mp3", "z.jpg", "t.txt"] {
            std::fs::write(d.join(n), "x").unwrap();
        }
        let names: Vec<String> = siblings(&d.join("b2.mp4")).iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, vec!["b2.mp4", "b10.mp4", "c.mp3"]);
        let imgs = siblings(&d.join("z.jpg"));
        assert_eq!(imgs.len(), 1);
        let _ = std::fs::remove_dir_all(&d);
    }
}

/// Famille du fichier produit par un profil (d'après son extension) : « gif » est traité comme une vidéo.
pub fn profile_class(p: &Profile) -> MediaKind {
    let e = p.extension.to_ascii_lowercase();
    if e == "gif" {
        MediaKind::Video
    } else if IMAGE_EXT.contains(&e.as_str()) {
        MediaKind::Image
    } else if AUDIO_EXT.contains(&e.as_str()) || p.is_audio_only() {
        MediaKind::Audio
    } else {
        MediaKind::Video
    }
}

/// Un profil est proposé pour une entrée de type `input` : vidéo -> vidéo ou audio (extraction),
/// audio -> audio, image -> image.
pub fn profile_accepts(p: &Profile, input: MediaKind) -> bool {
    match (input, profile_class(p)) {
        (MediaKind::Video, MediaKind::Video | MediaKind::Audio) => true,
        (a, b) => a == b,
    }
}
