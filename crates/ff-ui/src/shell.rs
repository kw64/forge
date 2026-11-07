//! Intégration à l'explorateur Windows (menu contextuel « Convertir avec MediaForge »), sans droits admin
//! (clés HKCU). Sous Linux, l'intégration passe par le fichier .desktop de l'AppImage. v0.3.0
use crate::util::MEDIA_EXT;
use anyhow::{bail, Result};
use std::path::Path;

pub const SUPPORTED: bool = cfg!(windows);

#[cfg(windows)]
fn reg(args: &[&str]) -> Result<()> {
    use std::os::windows::process::CommandExt;
    let out = std::process::Command::new("reg").args(args).creation_flags(0x0800_0000).output()?;
    if !out.status.success() {
        bail!("reg.exe : {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

#[cfg(windows)]
fn key(ext: &str) -> String {
    format!("HKCU\\Software\\Classes\\SystemFileAssociations\\.{ext}\\shell\\MediaForge")
}

/// Ajoute l'entrée de menu pour toutes les extensions média.
#[cfg(windows)]
pub fn install(exe: &Path, label: &str) -> Result<()> {
    let exe_s = exe.display().to_string();
    for ext in MEDIA_EXT {
        let k = key(ext);
        reg(&["add", &k, "/ve", "/d", label, "/f"])?;
        reg(&["add", &k, "/v", "Icon", "/d", &format!("\"{exe_s}\",0"), "/f"])?;
        reg(&["add", &format!("{k}\\command"), "/ve", "/d", &format!("\"{exe_s}\" \"%1\""), "/f"])?;
    }
    Ok(())
}

/// Retire l'entrée de menu.
#[cfg(windows)]
pub fn uninstall() -> Result<()> {
    for ext in MEDIA_EXT {
        let _ = reg(&["delete", &key(ext), "/f"]);
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn install(_exe: &Path, _label: &str) -> Result<()> {
    let _ = MEDIA_EXT;
    bail!("disponible uniquement sous Windows")
}

#[cfg(not(windows))]
pub fn uninstall() -> Result<()> {
    bail!("disponible uniquement sous Windows")
}
