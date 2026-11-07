//! Intégration à l'explorateur Windows : menu contextuel « Gérer avec FileForge » sur les dossiers,
//! le fond de dossier et les lecteurs (clés HKCU, sans droits admin). v0.14.0
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

/// (clé, argument transmis) : dossier (%1), fond de dossier (%V), lecteur (%1).
#[cfg(windows)]
const TARGETS: [(&str, &str); 3] = [
    ("HKCU\\Software\\Classes\\Directory\\shell\\FileForge", "%1"),
    ("HKCU\\Software\\Classes\\Directory\\Background\\shell\\FileForge", "%V"),
    ("HKCU\\Software\\Classes\\Drive\\shell\\FileForge", "%1"),
];

#[cfg(windows)]
pub fn install(exe: &Path, label: &str) -> Result<()> {
    let exe_s = exe.display().to_string();
    for (k, arg) in TARGETS {
        reg(&["add", k, "/ve", "/d", label, "/f"])?;
        reg(&["add", k, "/v", "Icon", "/d", &format!("\"{exe_s}\",0"), "/f"])?;
        reg(&["add", &format!("{k}\\command"), "/ve", "/d", &format!("\"{exe_s}\" \"{arg}\""), "/f"])?;
    }
    Ok(())
}

#[cfg(windows)]
pub fn uninstall() -> Result<()> {
    for (k, _) in TARGETS {
        let _ = reg(&["delete", k, "/f"]);
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn install(_exe: &Path, _label: &str) -> Result<()> {
    bail!("disponible uniquement sous Windows")
}

#[cfg(not(windows))]
pub fn uninstall() -> Result<()> {
    bail!("disponible uniquement sous Windows")
}
