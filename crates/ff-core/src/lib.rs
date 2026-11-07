//! ff-core : types partagés (profils, tâches, événements, arguments). Aucune dépendance UI ni FFmpeg. v0.4.0
pub mod args;
pub mod instance;
pub mod media;
pub mod rename;
pub mod spec;

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub type JobId = u64;

/// Langue de l'interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    #[default]
    Fr,
    En,
}

/// Accélération matérielle de l'encodage vidéo (repli logiciel automatique en cas d'échec).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HwAccel {
    #[default]
    None,
    /// Choisit le premier encodeur matériel disponible (NVENC, Quick Sync, AMF, VAAPI).
    Auto,
    Nvenc,
    Qsv,
    Amf,
    Vaapi,
}

impl HwAccel {
    pub const ALL: [HwAccel; 5] = [HwAccel::None, HwAccel::Nvenc, HwAccel::Qsv, HwAccel::Amf, HwAccel::Vaapi];
    /// Accélération réellement utilisée : `Auto` devient le meilleur encodeur disponible (ou le logiciel).
    pub fn resolve(self, avail: &[HwAccel]) -> HwAccel {
        if self != HwAccel::Auto {
            return self;
        }
        [HwAccel::Nvenc, HwAccel::Qsv, HwAccel::Amf, HwAccel::Vaapi].into_iter().find(|h| avail.contains(h)).unwrap_or(HwAccel::None)
    }
    pub fn label(self) -> &'static str {
        match self {
            HwAccel::None => "Logiciel (CPU)",
            HwAccel::Auto => "Automatique (GPU si disponible)",
            HwAccel::Amf => "AMD AMF",
            HwAccel::Nvenc => "NVIDIA NVENC",
            HwAccel::Qsv => "Intel Quick Sync",
            HwAccel::Vaapi => "VAAPI (Linux)",
        }
    }
    /// Suffixe des encodeurs FFmpeg (h264_nvenc, hevc_qsv…).
    pub fn suffix(self) -> &'static str {
        match self {
            HwAccel::None | HwAccel::Auto => "",
            HwAccel::Nvenc => "nvenc",
            HwAccel::Qsv => "qsv",
            HwAccel::Amf => "amf",
            HwAccel::Vaapi => "vaapi",
        }
    }
}

/// Recadrage en pixels (origine en haut à gauche).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Crop {
    pub w: u32,
    pub h: u32,
    pub x: u32,
    pub y: u32,
}

/// Retouches rapides appliquées pendant la conversion (mini-éditeur).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Edits {
    pub trim_start: Option<f64>,
    pub trim_end: Option<f64>,
    pub crop: Option<Crop>,
}

impl Edits {
    pub fn is_empty(&self) -> bool {
        self.trim_start.is_none() && self.trim_end.is_none() && self.crop.is_none()
    }
    /// Durée conservée après découpe, si elle peut être connue.
    pub fn span(&self, total: Option<f64>) -> Option<f64> {
        let start = self.trim_start.unwrap_or(0.0);
        match (self.trim_end, total) {
            (Some(e), _) => Some((e - start).max(0.0)),
            (None, Some(t)) => Some((t - start).max(0.0)),
            _ => None,
        }
    }
}

/// Nature de la tâche.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum JobKind {
    #[default]
    Convert,
    /// Fusion bout à bout (avec ou sans réencodage).
    Concat { inputs: Vec<PathBuf>, copy: bool },
    /// Ajoute une piste audio à une vidéo (le flux vidéo est copié).
    Mux { audio: PathBuf },
    /// Mélange plusieurs pistes audio en une seule.
    Mix { inputs: Vec<PathBuf> },
}

/// Profil d'encodage : extension de sortie + arguments FFmpeg (entre l'entrée et la sortie).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    pub label: String,
    pub extension: String,
    pub args: Vec<String>,
    /// Profil intégré (non modifiable). Les profils utilisateur sont à `false`.
    #[serde(default)]
    pub builtin: bool,
    /// Profil créé avec l'éditeur visuel : `args` est alors dérivé de ce modèle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spec: Option<spec::VideoSpec>,
}

impl Profile {
    fn new(id: &str, label: &str, ext: &str, args: &[&str]) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            extension: ext.into(),
            args: args.iter().map(|s| s.to_string()).collect(),
            builtin: true,
            spec: None,
        }
    }
    /// Vrai si le profil produit de l'audio seul.
    pub fn is_audio_only(&self) -> bool {
        self.args.iter().any(|a| a == "-vn")
    }
    /// Vrai si le profil produit une image fixe.
    pub fn is_image(&self) -> bool {
        matches!(self.extension.as_str(), "jpg" | "jpeg" | "png" | "webp" | "bmp" | "tiff" | "avif")
    }
}

/// Profils intégrés (vidéo et audio).
pub fn builtin_profiles() -> Vec<Profile> {
    vec![
        Profile::new("mp4", "Vidéo MP4 (H.264 + AAC)", "mp4",
            &["-c:v", "libx264", "-preset", "medium", "-crf", "23", "-c:a", "aac", "-b:a", "128k", "-movflags", "+faststart"]),
        Profile::new("mp4-small", "Vidéo MP4 légère (H.264, CRF 28)", "mp4",
            &["-c:v", "libx264", "-preset", "medium", "-crf", "28", "-vf", "scale='min(1280,iw)':-2", "-c:a", "aac", "-b:a", "96k", "-movflags", "+faststart"]),
        Profile::new("webm", "Vidéo WebM (VP9 + Opus)", "webm",
            &["-c:v", "libvpx-vp9", "-crf", "32", "-b:v", "0", "-c:a", "libopus", "-b:a", "96k"]),
        Profile::new("mkv", "Vidéo MKV (copie des flux)", "mkv", &["-c", "copy"]),
        // HDLight : encodages MKV compacts (qualité proche du Blu-ray à 1/4–1/10 du poids). Toutes les pistes audio et
        // sous-titres sont conservées (sous-titres copiés), chapitres et métadonnées aussi.
        Profile::new("hdlight-1080-x264", "HDLight 1080p MKV (x264 CRF 20, AAC 256k)", "mkv",
            &["-map", "0:v:0", "-map", "0:a?", "-map", "0:s?", "-c:v", "libx264", "-preset", "slow", "-crf", "20", "-profile:v", "high", "-pix_fmt", "yuv420p",
              "-vf", "scale='min(1920,iw)':-2:flags=lanczos", "-c:a", "aac", "-b:a", "256k", "-c:s", "copy", "-max_muxing_queue_size", "1024"]),
        Profile::new("hdlight-1080-x265", "HDLight 1080p MKV (x265 10 bits CRF 23, AAC 256k)", "mkv",
            &["-map", "0:v:0", "-map", "0:a?", "-map", "0:s?", "-c:v", "libx265", "-preset", "medium", "-crf", "23", "-pix_fmt", "yuv420p10le",
              "-vf", "scale='min(1920,iw)':-2:flags=lanczos", "-c:a", "aac", "-b:a", "256k", "-c:s", "copy", "-max_muxing_queue_size", "1024"]),
        Profile::new("hdlight-720-x264", "HDLight 720p MKV (x264 CRF 21, AAC 192k)", "mkv",
            &["-map", "0:v:0", "-map", "0:a?", "-map", "0:s?", "-c:v", "libx264", "-preset", "slow", "-crf", "21", "-profile:v", "high", "-pix_fmt", "yuv420p",
              "-vf", "scale='min(1280,iw)':-2:flags=lanczos", "-c:a", "aac", "-b:a", "192k", "-c:s", "copy", "-max_muxing_queue_size", "1024"]),
        Profile::new("hdlight-720-x265", "HDLight 720p MKV (x265 10 bits CRF 24, AAC 192k)", "mkv",
            &["-map", "0:v:0", "-map", "0:a?", "-map", "0:s?", "-c:v", "libx265", "-preset", "medium", "-crf", "24", "-pix_fmt", "yuv420p10le",
              "-vf", "scale='min(1280,iw)':-2:flags=lanczos", "-c:a", "aac", "-b:a", "192k", "-c:s", "copy", "-max_muxing_queue_size", "1024"]),
        Profile::new("hdlight-mini", "HDLight mini 720p MKV (x265 10 bits CRF 27, AAC 128k)", "mkv",
            &["-map", "0:v:0", "-map", "0:a?", "-map", "0:s?", "-c:v", "libx265", "-preset", "medium", "-crf", "27", "-pix_fmt", "yuv420p10le",
              "-vf", "scale='min(1280,iw)':-2:flags=lanczos", "-c:a", "aac", "-b:a", "128k", "-c:s", "copy", "-max_muxing_queue_size", "1024"]),
        Profile::new("mp3", "Audio MP3 (192 kbit/s)", "mp3", &["-vn", "-c:a", "libmp3lame", "-b:a", "192k"]),
        Profile::new("aac", "Audio AAC / M4A (160 kbit/s)", "m4a", &["-vn", "-c:a", "aac", "-b:a", "160k"]),
        Profile::new("ogg", "Audio OGG Opus (128 kbit/s)", "ogg", &["-vn", "-c:a", "libopus", "-b:a", "128k"]),
        Profile::new("flac", "Audio FLAC (sans perte)", "flac", &["-vn", "-c:a", "flac"]),
        Profile::new("wav", "Audio WAV (PCM 16 bits)", "wav", &["-vn", "-c:a", "pcm_s16le"]),
        Profile::new("gif", "GIF animé (480 px, 12 i/s)", "gif", &["-vf", "fps=12,scale=480:-1:flags=lanczos", "-an"]),
        Profile::new("jpg", "Image JPEG (qualité 90)", "jpg", &["-frames:v", "1", "-q:v", "3", "-an"]),
        Profile::new("png", "Image PNG (sans perte)", "png", &["-frames:v", "1", "-an"]),
        Profile::new("webp", "Image WebP (qualité 85)", "webp", &["-frames:v", "1", "-c:v", "libwebp", "-quality", "85", "-an"]),
        Profile::new("bmp", "Image BMP", "bmp", &["-frames:v", "1", "-an"]),
        Profile::new("tiff", "Image TIFF", "tiff", &["-frames:v", "1", "-an"]),
        Profile::new("avif", "Image AVIF (qualité moyenne)", "avif", &["-frames:v", "1", "-c:v", "libaom-av1", "-crf", "30", "-b:v", "0", "-cpu-used", "6", "-an"]),
    ]
}

/// Retrouve un profil intégré par son identifiant.
pub fn builtin_by_id(id: &str) -> Option<Profile> {
    builtin_profiles().into_iter().find(|p| p.id == id)
}

/// Tâche de conversion.
#[derive(Debug, Clone)]
pub struct Job {
    pub input: PathBuf,
    pub output: PathBuf,
    pub profile: Profile,
    pub kind: JobKind,
    pub edits: Edits,
    pub hw: HwAccel,
}

impl Job {
    /// Conversion simple, sans retouche ni accélération.
    pub fn convert(input: PathBuf, output: PathBuf, profile: Profile) -> Self {
        Self { input, output, profile, kind: JobKind::Convert, edits: Edits::default(), hw: HwAccel::None }
    }
}

/// Progression d'une tâche.
#[derive(Debug, Clone, Copy, Default)]
pub struct Progress {
    /// 0.0 à 100.0 (0 si la durée est inconnue).
    pub percent: f32,
    /// Temps média traité, en secondes.
    pub out_time_secs: f64,
    /// Vitesse relative au temps réel (ex. 2.5 = 2,5x).
    pub speed: f32,
}

/// Événements émis par la file d'attente.
#[derive(Debug, Clone)]
pub enum Event {
    Started(JobId),
    Progress(JobId, Progress),
    Done(JobId),
    Failed(JobId, String),
    Cancelled(JobId),
    /// Information non bloquante (ex. repli logiciel).
    Info(JobId, String),
}

/// Chemin de sortie par défaut : dossier choisi (ou celui de la source), extension du profil.
pub fn default_output(input: &Path, out_dir: Option<&Path>, ext: &str) -> PathBuf {
    let stem = input.file_stem().map(|s| s.to_os_string()).unwrap_or_else(|| "sortie".into());
    let dir = out_dir
        .map(Path::to_path_buf)
        .or_else(|| input.parent().map(Path::to_path_buf))
        .unwrap_or_default();
    let mut name = stem;
    name.push(".");
    name.push(ext);
    dir.join(name)
}

/// Évite d'écraser un fichier existant ou l'entrée : ajoute « (1) », « (2) »…
pub fn unique_output(path: PathBuf, input: &Path) -> PathBuf {
    if !path.exists() && path != input {
        return path;
    }
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = path.extension().map(|s| s.to_string_lossy().into_owned());
    let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
    for n in 1..10_000 {
        let name = match &ext {
            Some(e) => format!("{stem} ({n}).{e}"),
            None => format!("{stem} ({n})"),
        };
        let cand = dir.join(name);
        if !cand.exists() && cand != input {
            return cand;
        }
    }
    path
}

/// Découpe une ligne d'arguments en respectant les guillemets simples et doubles.
pub fn split_args(line: &str) -> Vec<String> {
    let (mut out, mut cur, mut quote, mut has) = (Vec::new(), String::new(), None::<char>, false);
    for c in line.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => cur.push(c),
            (None, '"') | (None, '\'') => {
                quote = Some(c);
                has = true;
            }
            (None, c) if c.is_whitespace() => {
                if has || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                    has = false;
                }
            }
            (None, c) => cur.push(c),
        }
    }
    if has || !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Normalise un identifiant de profil utilisateur.
pub fn slug(label: &str) -> String {
    let s: String = label
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect();
    let s = s.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-");
    if s.is_empty() { "profil".into() } else { s }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn split_quotes() {
        assert_eq!(split_args(r#"-vf "scale=1280:-2" -c:a aac"#), vec!["-vf", "scale=1280:-2", "-c:a", "aac"]);
        assert_eq!(split_args("  "), Vec::<String>::new());
    }
    #[test]
    fn unique_never_returns_input() {
        let i = Path::new("/tmp/zz_nonexistent_in.mp4");
        assert_ne!(unique_output(i.to_path_buf(), i), i);
    }
    #[test]
    fn slugging() {
        assert_eq!(slug("Mon profil 4K!"), "mon-profil-4k");
        assert_eq!(slug("???"), "profil");
    }
}
