//! FFmpeg « full » (encodeurs GPU : NVENC, AMF, Quick Sync) : localisation et installation dans le dossier de données.
//! La version « lite » reste celle détectée par ff-engine (réglage, dossier ffmpeg/, PATH). v0.14.0
use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(windows)]
const URL: &str = "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip";
#[cfg(not(windows))]
const URL: &str = "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-linux64-gpl.tar.xz";

fn exe(n: &str) -> String {
    if cfg!(windows) { format!("{n}.exe") } else { n.to_string() }
}

/// Dossier d'installation par défaut de la version full.
pub fn default_dir() -> PathBuf {
    ff_profiles::data_dir().join("ffmpeg-full")
}

/// Dossier contenant ffmpeg et ffprobe fonctionnels : `custom` (réglage) puis le dossier par défaut.
pub fn locate_full(custom: Option<&Path>) -> Option<PathBuf> {
    let ok = |d: &Path| [exe("ffmpeg"), exe("ffprobe")].iter().all(|n| super::silent(Command::new(d.join(n)).arg("-version")).output().map(|o| o.status.success()).unwrap_or(false));
    custom.map(|p| p.to_path_buf()).into_iter().chain(std::iter::once(default_dir())).find(|d| ok(d))
}

/// Cherche récursivement `name` sous `dir`.
fn find(dir: &Path, name: &str) -> Option<PathBuf> {
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        let p = e.path();
        if p.is_dir() {
            if let Some(f) = find(&p, name) {
                return Some(f);
            }
        } else if p.file_name().map(|n| n == name).unwrap_or(false) {
            return Some(p);
        }
    }
    None
}

/// Télécharge (curl) et installe FFmpeg full dans `default_dir()` ; renvoie ce dossier.
/// Archive extraite avec `tar` (livré avec Windows 10+ et Linux).
pub fn install_full() -> Result<PathBuf, String> {
    let dest = default_dir();
    let work = dest.with_extension("tmp");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let res = (|| {
        let arc = work.join(if cfg!(windows) { "ffmpeg.zip" } else { "ffmpeg.tar.xz" });
        let o = super::silent(Command::new("curl").args(["-L", "--fail", "-sS", "-o"]).arg(&arc).arg(URL)).output().map_err(|e| format!("curl : {e}"))?;
        if !o.status.success() {
            return Err(String::from_utf8_lossy(&o.stderr).trim().to_string());
        }
        let ex = work.join("x");
        std::fs::create_dir_all(&ex).map_err(|e| e.to_string())?;
        let o = super::silent(Command::new("tar").arg("-xf").arg(&arc).arg("-C").arg(&ex)).output().map_err(|e| format!("tar : {e}"))?;
        if !o.status.success() {
            return Err(String::from_utf8_lossy(&o.stderr).trim().to_string());
        }
        let (ff, fp) = (find(&ex, &exe("ffmpeg")).ok_or("ffmpeg introuvable dans l'archive")?, find(&ex, &exe("ffprobe")).ok_or("ffprobe introuvable dans l'archive")?);
        let _ = std::fs::remove_dir_all(&dest);
        std::fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
        for p in [&ff, &fp] {
            std::fs::copy(p, dest.join(p.file_name().unwrap())).map_err(|e| e.to_string())?;
        }
        Ok(dest.clone())
    })();
    let _ = std::fs::remove_dir_all(&work);
    res
}
