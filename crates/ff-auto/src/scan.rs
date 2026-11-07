//! Analyse des fichiers : type, dates, EXIF, tags audio. v0.1.0
use crate::job::Job;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub enum Kind {
    Photo,
    Video,
    Audio,
    Doc,
    Archive,
    Rom,
    Other,
}

impl Kind {
    pub const ALL: [Kind; 7] = [Kind::Photo, Kind::Video, Kind::Audio, Kind::Doc, Kind::Archive, Kind::Rom, Kind::Other];
    pub fn label(self) -> &'static str {
        match self {
            Kind::Photo => "Photos",
            Kind::Video => "Vidéos",
            Kind::Audio => "Musique",
            Kind::Doc => "Documents",
            Kind::Archive => "Archives",
            Kind::Rom => "Jeux (ROMs)",
            Kind::Other => "Autres",
        }
    }
    pub fn key(self) -> &'static str {
        match self {
            Kind::Photo => "photo",
            Kind::Video => "video",
            Kind::Audio => "audio",
            Kind::Doc => "document",
            Kind::Archive => "archive",
            Kind::Rom => "rom",
            Kind::Other => "autre",
        }
    }
    pub fn from_key(s: &str) -> Option<Kind> {
        let s = s.to_lowercase();
        Kind::ALL.into_iter().find(|k| k.key() == s || k.label().to_lowercase() == s || (s.starts_with("image") && *k == Kind::Photo) || (s.starts_with("vid") && *k == Kind::Video) || (s.starts_with("mus") && *k == Kind::Audio) || (s.starts_with("doc") && *k == Kind::Doc))
    }
}

const PHOTO: &[&str] = &["jpg", "jpeg", "png", "gif", "bmp", "webp", "tif", "tiff", "heic", "heif", "avif", "raw", "cr2", "cr3", "nef", "arw", "dng", "orf", "rw2"];
const VIDEO: &[&str] = &["mp4", "mkv", "avi", "mov", "wmv", "flv", "webm", "m4v", "mpg", "mpeg", "ts", "m2ts", "3gp", "ogv"];
const AUDIO: &[&str] = &["mp3", "flac", "wav", "ogg", "opus", "m4a", "aac", "wma", "aiff", "ape", "mid", "midi", "mod", "xm", "it", "s3m"];
const DOC: &[&str] = &["pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt", "ods", "odp", "txt", "rtf", "md", "csv", "epub", "html", "htm"];
const ARCH: &[&str] = &["zip", "rar", "7z", "tar", "gz", "tgz", "bz2", "xz", "lha", "lzh"];
const ROM: &[&str] = &["nes", "sfc", "smc", "fig", "gb", "gbc", "gba", "nds", "3ds", "md", "gen", "smd", "sms", "gg", "pce", "z64", "n64", "v64", "a26", "a78", "lnx", "ws", "wsc", "ngp", "ngc", "adf", "dms", "d64", "t64", "tap", "tzx", "dsk", "st", "stx", "iso", "cue", "chd", "rom", "col", "int", "vec", "32x", "sg", "sc"];

pub fn ext_of(p: &Path) -> String {
    p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

pub fn kind_of_ext(e: &str) -> Kind {
    let has = |l: &[&str]| l.contains(&e);
    if has(PHOTO) {
        Kind::Photo
    } else if has(VIDEO) {
        Kind::Video
    } else if has(AUDIO) {
        Kind::Audio
    } else if has(ROM) {
        Kind::Rom
    } else if has(DOC) {
        Kind::Doc
    } else if has(ARCH) {
        Kind::Archive
    } else {
        Kind::Other
    }
}

#[derive(Clone, Debug, Default)]
pub struct Facts {
    pub path: PathBuf,
    pub size: u64,
    pub mtime: i64,
    pub ext: String,
    pub kind: Option<Kind>,
    /// Date de prise de vue (EXIF) en secondes Unix, sinon None.
    pub taken: Option<i64>,
    pub camera: String,
    pub artist: String,
    pub album: String,
    pub title: String,
    pub track: u32,
    pub year_tag: u32,
}

impl Facts {
    pub fn kind(&self) -> Kind {
        self.kind.unwrap_or(Kind::Other)
    }
    /// Date de référence : prise de vue si connue, sinon modification.
    pub fn date(&self) -> (i32, u32, u32) {
        crate::civil(self.taken.unwrap_or(self.mtime))
    }
    pub fn stem(&self) -> String {
        self.path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()
    }
    pub fn name(&self) -> String {
        self.path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()
    }
}

fn parse_exif_date(s: &str) -> Option<i64> {
    let b: Vec<u32> = s.split(|c: char| !c.is_ascii_digit()).filter(|x| !x.is_empty()).filter_map(|x| x.parse().ok()).collect();
    if b.len() >= 3 && b[0] > 1900 && (1..=12).contains(&b[1]) && (1..=31).contains(&b[2]) {
        let t = crate::epoch(b[0] as i32, b[1], b[2]) + b.get(3).copied().unwrap_or(0) as i64 * 3600 + b.get(4).copied().unwrap_or(0) as i64 * 60 + b.get(5).copied().unwrap_or(0) as i64;
        return Some(t);
    }
    None
}

/// Date devinée depuis le nom (IMG_20230714_…, 2023-07-14 …, PXL_2023…).
pub fn date_from_name(name: &str) -> Option<i64> {
    let re = regex::Regex::new(r"((?:19|20)\d{2})[-_.]?(0[1-9]|1[0-2])[-_.]?(0[1-9]|[12]\d|3[01])").ok()?;
    let c = re.captures(name)?;
    Some(crate::epoch(c[1].parse().ok()?, c[2].parse().ok()?, c[3].parse().ok()?))
}

/// Lit les métadonnées d'un fichier (`deep` : EXIF / tags audio).
pub fn facts(p: &Path, deep: bool) -> Option<Facts> {
    let md = std::fs::metadata(p).ok()?;
    if !md.is_file() {
        return None;
    }
    let mtime = md.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64).unwrap_or(0);
    let ext = ext_of(p);
    let kind = kind_of_ext(&ext);
    let mut f = Facts { path: p.to_path_buf(), size: md.len(), mtime, ext, kind: Some(kind), ..Default::default() };
    if deep {
        match kind {
            Kind::Photo => {
                if let Ok(file) = std::fs::File::open(p) {
                    if let Ok(ex) = exif::Reader::new().read_from_container(&mut std::io::BufReader::new(file)) {
                        let get = |t| ex.get_field(t, exif::In::PRIMARY).map(|x| x.display_value().to_string());
                        f.taken = get(exif::Tag::DateTimeOriginal).or_else(|| get(exif::Tag::DateTime)).and_then(|s| parse_exif_date(&s));
                        let make = get(exif::Tag::Make).unwrap_or_default();
                        let model = get(exif::Tag::Model).unwrap_or_default();
                        f.camera = format!("{} {}", make.trim_matches('"').trim(), model.trim_matches('"').trim()).trim().to_string();
                    }
                }
                if f.taken.is_none() {
                    f.taken = date_from_name(&f.name());
                }
            }
            Kind::Video => f.taken = date_from_name(&f.name()),
            Kind::Audio => {
                use lofty::prelude::*;
                if let Ok(tf) = lofty::read_from_path(p) {
                    if let Some(t) = tf.primary_tag().or_else(|| tf.first_tag()) {
                        f.artist = t.artist().map(|s| s.to_string()).unwrap_or_default();
                        f.album = t.album().map(|s| s.to_string()).unwrap_or_default();
                        f.title = t.title().map(|s| s.to_string()).unwrap_or_default();
                        f.track = t.track().unwrap_or(0);
                        f.year_tag = t.year().unwrap_or(0);
                    }
                }
            }
            _ => {}
        }
    }
    Some(f)
}

/// Parcourt un dossier (récursif en option) ; annulable.
pub fn walk(root: &Path, recurse: bool, job: Option<&Job>) -> Vec<PathBuf> {
    let mut out = vec![];
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                let n = e.file_name().to_string_lossy().to_string();
                if recurse && !n.starts_with('.') && n != "_Doublons" && n != "$RECYCLE.BIN" && n != "System Volume Information" {
                    stack.push(p);
                }
            } else if ft.is_file() {
                out.push(p);
            }
        }
        if let Some(j) = job {
            if !j.step(0) {
                break;
            }
            j.set_detail(format!("{} fichiers…", out.len()));
        }
    }
    out
}

/// Analyse complète d'une liste de fichiers (avec progression).
pub fn scan(paths: &[PathBuf], deep: bool, job: &Job) -> Vec<Facts> {
    job.reset(paths.len() as u64);
    let mut v = Vec::with_capacity(paths.len());
    for p in paths {
        if !job.step(1) {
            break;
        }
        if let Some(f) = facts(p, deep) {
            v.push(f);
        }
    }
    v
}
