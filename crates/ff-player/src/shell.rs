//! Intégration Windows : association des formats (HKCU, sans droits admin) et ouverture des
//! « Applications par défaut » où l'utilisateur choisit MediaForge Player. v0.4.0
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

/// Déclare le lecteur (ProgID, capacités, applications enregistrées) puis ouvre les réglages Windows.
#[cfg(windows)]
pub fn register(exe: &Path) -> Result<()> {
    use ff_core::media::{AUDIO_EXT, IMAGE_EXT, VIDEO_EXT};
    use std::os::windows::process::CommandExt;
    let exe_s = exe.display().to_string();
    let prog = "HKCU\\Software\\Classes\\MediaForgePlayer.File";
    reg(&["add", prog, "/ve", "/d", "Fichier média (MediaForge Player)", "/f"])?;
    reg(&["add", &format!("{prog}\\DefaultIcon"), "/ve", "/d", &format!("\"{exe_s}\",0"), "/f"])?;
    reg(&["add", &format!("{prog}\\shell\\open\\command"), "/ve", "/d", &format!("\"{exe_s}\" \"%1\""), "/f"])?;
    let cap = "HKCU\\Software\\MediaForge\\Player\\Capabilities";
    reg(&["add", cap, "/v", "ApplicationName", "/d", "MediaForge Player", "/f"])?;
    reg(&["add", cap, "/v", "ApplicationDescription", "/d", "Lecteur audio, vidéo et images", "/f"])?;
    for ext in VIDEO_EXT.iter().chain(AUDIO_EXT).chain(IMAGE_EXT) {
        reg(&["add", &format!("{cap}\\FileAssociations"), "/v", &format!(".{ext}"), "/d", "MediaForgePlayer.File", "/f"])?;
        reg(&["add", &format!("HKCU\\Software\\Classes\\.{ext}\\OpenWithProgids"), "/v", "MediaForgePlayer.File", "/t", "REG_NONE", "/f"])?;
    }
    reg(&["add", "HKCU\\Software\\RegisteredApplications", "/v", "MediaForgePlayer", "/d", "Software\\MediaForge\\Player\\Capabilities", "/f"])?;
    let _ = std::process::Command::new("cmd").args(["/c", "start", "", "ms-settings:defaultapps"]).creation_flags(0x0800_0000).spawn();
    Ok(())
}

#[cfg(not(windows))]
pub fn register(_exe: &Path) -> Result<()> {
    bail!("disponible uniquement sous Windows")
}

/// Lancement avec Windows (clé HKCU\\...\\Run, sans droits admin), démarrage réduit.
#[cfg(windows)]
pub fn set_autostart(exe: &Path, on: bool) -> Result<()> {
    let k = "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run";
    if on {
        reg(&["add", k, "/v", "MediaForgePlayer", "/d", &format!("\"{}\" --minimized", exe.display()), "/f"])
    } else {
        let _ = reg(&["delete", k, "/v", "MediaForgePlayer", "/f"]);
        Ok(())
    }
}

#[cfg(not(windows))]
pub fn set_autostart(_exe: &Path, _on: bool) -> Result<()> {
    bail!("disponible uniquement sous Windows")
}
