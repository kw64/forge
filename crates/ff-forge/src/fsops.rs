//! Opérations disque de base. v0.6.0
use crate::jobs::Job;
use std::io::{Read, Write};
use std::path::Path;

pub fn copy_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    if src.is_dir() {
        std::fs::create_dir_all(dst).map_err(|e| e.to_string())?;
        for e in std::fs::read_dir(src).map_err(|e| e.to_string())?.flatten() {
            copy_recursive(&e.path(), &dst.join(e.file_name()))?;
        }
        Ok(())
    } else {
        if let Some(d) = dst.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        std::fs::copy(src, dst).map(|_| ()).map_err(|e| format!("{}: {e}", src.display()))
    }
}

pub fn remove(p: &Path) -> Result<(), String> {
    if p.is_dir() { std::fs::remove_dir_all(p) } else { std::fs::remove_file(p) }.map_err(|e| format!("{}: {e}", p.display()))
}

/// Déplace (renommage, ou copie + suppression entre volumes).
#[allow(dead_code)]
pub fn move_to(src: &Path, dst: &Path) -> Result<(), String> {
    if std::fs::rename(src, dst).is_ok() {
        return Ok(());
    }
    copy_recursive(src, dst)?;
    remove(src)
}

pub fn dir_size(p: &Path) -> (u64, u64, u64) {
    // (octets, fichiers, dossiers)
    let mut t = (0, 0, 0);
    if let Ok(rd) = std::fs::read_dir(p) {
        for e in rd.flatten() {
            let m = match e.metadata() { Ok(m) => m, Err(_) => continue };
            if m.is_dir() {
                let s = dir_size(&e.path());
                t = (t.0 + s.0, t.1 + s.1, t.2 + s.2 + 1);
            } else {
                t = (t.0 + m.len(), t.1 + 1, t.2);
            }
        }
    }
    t
}

/// Nombre avec séparateur de milliers à l'espace : 10 474.
pub fn spaced(n: u64) -> String {
    let s = n.to_string();
    let mut o = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            o.push(' ');
        }
        o.push(c);
    }
    o
}

/// Taille totale (octets) d'un fichier ou d'une arborescence.
pub fn tree_size(p: &Path) -> u64 {
    match std::fs::symlink_metadata(p) {
        Ok(m) if m.is_dir() => std::fs::read_dir(p).map(|rd| rd.flatten().map(|e| tree_size(&e.path())).sum()).unwrap_or(0),
        Ok(m) => m.len(),
        Err(_) => 0,
    }
}

const BIG: u64 = 8 * 1024 * 1024;

fn copy_file_p(src: &Path, dst: &Path, job: &Job) -> Result<(), String> {
    let md = std::fs::metadata(src).map_err(|e| format!("{}: {e}", src.display()))?;
    if let Some(d) = dst.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    if md.len() < BIG {
        std::fs::copy(src, dst).map_err(|e| format!("{}: {e}", src.display()))?;
        return if job.step(md.len()) { Ok(()) } else { Err("Annulé".into()) };
    }
    // Gros fichier : copie par blocs pour suivre la progression et pouvoir annuler.
    let mut r = std::fs::File::open(src).map_err(|e| format!("{}: {e}", src.display()))?;
    let mut w = std::fs::File::create(dst).map_err(|e| format!("{}: {e}", dst.display()))?;
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = r.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        w.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        if !job.step(n as u64) {
            drop(w);
            let _ = std::fs::remove_file(dst);
            return Err("Annulé".into());
        }
    }
    if let Ok(t) = md.modified() {
        let _ = w.set_modified(t);
    }
    Ok(())
}

/// Copie récursive avec progression (octets) et annulation.
pub fn copy_p(src: &Path, dst: &Path, job: &Job) -> Result<(), String> {
    if src.is_dir() {
        std::fs::create_dir_all(dst).map_err(|e| e.to_string())?;
        for e in std::fs::read_dir(src).map_err(|e| e.to_string())?.flatten() {
            copy_p(&e.path(), &dst.join(e.file_name()), job)?;
        }
        Ok(())
    } else {
        job.set_detail(src.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
        copy_file_p(src, dst, job)
    }
}

/// Déplacement avec progression : renommage instantané, sinon copie + suppression.
pub fn move_p(src: &Path, dst: &Path, job: &Job) -> Result<(), String> {
    let size = tree_size(src);
    if std::fs::rename(src, dst).is_ok() {
        job.step(size);
        return Ok(());
    }
    copy_p(src, dst, job)?;
    remove(src)
}

/// Taille en Mo (méga-octets, 1 Mo = 1 048 576 octets), 2 décimales.
pub fn mo(b: u64, en: bool) -> String {
    let v = format!("{:.2}", b as f64 / 1_048_576.0);
    format!("{} {}", if en { v } else { v.replace('.', ",") }, if en { "MB" } else { "Mo" })
}
