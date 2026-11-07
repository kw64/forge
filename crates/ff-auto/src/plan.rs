//! Plan de rangement : éléments modifiables, doublons, exécution annulable (journal). v0.1.0
use crate::cfg::Config;
use crate::job::Job;
use crate::rules;
use crate::scan::Facts;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Origin {
    Rule,
    Ai,
    Web,
    Cmd,
    Dup,
}

impl Origin {
    pub fn label(self) -> &'static str {
        match self {
            Origin::Rule => "Règle",
            Origin::Ai => "IA",
            Origin::Web => "Web/DAT",
            Origin::Cmd => "Commande",
            Origin::Dup => "Doublon",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Item {
    pub src: PathBuf,
    pub dst: PathBuf,
    pub reason: String,
    pub origin: Origin,
    pub checked: bool,
}

/// Plan de tri par règles : chaque fichier vers `dest/<gabarit>/<nom>`, doublons mis de côté.
pub fn build(files: &[Facts], cfg: &Config, dest: &Path, job: &Job) -> Vec<Item> {
    let mut items = vec![];
    job.reset(files.len() as u64);
    for f in files {
        if !job.step(1) {
            break;
        }
        let Some(r) = rules::first_match(&cfg.rules, f) else { continue };
        let dir = rules::render(&r.template, f, &[]);
        let dst = dest.join(&dir).join(f.name());
        if dst != f.path {
            items.push(Item { src: f.path.clone(), dst, reason: format!("règle « {} »", r.name), origin: Origin::Rule, checked: true });
        }
    }
    if cfg.dups {
        mark_duplicates(files, &mut items, dest, job);
    }
    items
}

/// Doublons exacts (taille puis SHA-1) : les copies supplémentaires vont dans `dest/_Doublons`, jamais supprimées.
pub fn mark_duplicates(files: &[Facts], items: &mut Vec<Item>, dest: &Path, job: &Job) {
    let mut by_size: HashMap<u64, Vec<&Facts>> = HashMap::new();
    for f in files.iter().filter(|f| f.size > 0) {
        by_size.entry(f.size).or_default().push(f);
    }
    let groups: Vec<_> = by_size.into_values().filter(|g| g.len() > 1).collect();
    job.reset(groups.iter().map(|g| g.len() as u64).sum());
    job.set_detail("Recherche de doublons…");
    for g in groups {
        let mut by_hash: HashMap<String, Vec<&Facts>> = HashMap::new();
        for f in g {
            if !job.step(1) {
                return;
            }
            if let Some(h) = sha1_file(&f.path) {
                by_hash.entry(h).or_default().push(f);
            }
        }
        for (_, mut same) in by_hash.into_iter().filter(|(_, v)| v.len() > 1) {
            same.sort_by(|a, b| a.mtime.cmp(&b.mtime).then(a.path.as_os_str().len().cmp(&b.path.as_os_str().len())));
            let keep = same[0].path.clone();
            for d in &same[1..] {
                let dst = dest.join("_Doublons").join(d.name());
                let reason = format!("doublon de {}", keep.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
                // Remplace une éventuelle entrée de tri existante pour ce fichier.
                if let Some(it) = items.iter_mut().find(|i| i.src == d.path) {
                    it.dst = dst;
                    it.reason = reason;
                    it.origin = Origin::Dup;
                } else {
                    items.push(Item { src: d.path.clone(), dst, reason, origin: Origin::Dup, checked: true });
                }
            }
        }
    }
}

pub fn sha1_file(p: &Path) -> Option<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(p).ok()?;
    let mut h = sha1_smol::Sha1::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Some(h.digest().to_string())
}

/// Chemin libre : ajoute « (2) », « (3) »… si la cible existe.
pub fn unique(p: &Path) -> PathBuf {
    if !p.exists() {
        return p.to_path_buf();
    }
    let stem = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let ext = p.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    for n in 2.. {
        let c = p.with_file_name(format!("{stem} ({n}){ext}"));
        if !c.exists() {
            return c;
        }
    }
    unreachable!()
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Op {
    pub from: PathBuf,
    pub to: PathBuf,
    pub copied: bool,
}

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct Journal {
    pub when: u64,
    pub ops: Vec<Op>,
}

fn journal_dir() -> PathBuf {
    ff_profiles::data_dir().join("autoforge_undo")
}

fn move_file(a: &Path, b: &Path) -> std::io::Result<()> {
    if let Some(d) = b.parent() {
        std::fs::create_dir_all(d)?;
    }
    if std::fs::rename(a, b).is_ok() {
        return Ok(());
    }
    std::fs::copy(a, b)?;
    std::fs::remove_file(a)
}

/// Exécute les éléments cochés ; écrit un journal pour l'annulation. Retourne (journal, erreurs).
pub fn apply(items: &[Item], copy: bool, job: &Job) -> (Journal, Vec<String>) {
    let todo: Vec<&Item> = items.iter().filter(|i| i.checked && i.src != i.dst).collect();
    job.reset(todo.len() as u64);
    let mut j = Journal { when: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0), ops: vec![] };
    let mut errs = vec![];
    for it in todo {
        if !job.step(1) {
            break;
        }
        job.set_detail(it.src.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
        if !it.src.exists() {
            errs.push(format!("{} : introuvable", it.src.display()));
            continue;
        }
        let to = unique(&it.dst);
        let r = if copy {
            to.parent().map(std::fs::create_dir_all).transpose().and_then(|_| std::fs::copy(&it.src, &to)).map(|_| ())
        } else {
            move_file(&it.src, &to)
        };
        match r {
            Ok(()) => j.ops.push(Op { from: it.src.clone(), to, copied: copy }),
            Err(e) => errs.push(format!("{} : {e}", it.src.display())),
        }
    }
    if !j.ops.is_empty() {
        let _ = std::fs::create_dir_all(journal_dir());
        let _ = std::fs::write(journal_dir().join(format!("{}.json", j.when)), serde_json::to_vec(&j).unwrap_or_default());
    }
    (j, errs)
}

/// Dernier journal d'annulation disponible.
pub fn last_journal() -> Option<(PathBuf, Journal)> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(journal_dir()).ok()?.flatten().map(|e| e.path()).filter(|p| p.extension().map(|e| e == "json").unwrap_or(false)).collect();
    v.sort();
    let p = v.pop()?;
    let j = serde_json::from_slice(&std::fs::read(&p).ok()?).ok()?;
    Some((p, j))
}

/// Annule un journal : remet les fichiers déplacés à leur place (les copies créées sont retirées).
pub fn undo(path: &Path, j: &Journal, job: &Job) -> (usize, Vec<String>) {
    job.reset(j.ops.len() as u64);
    let (mut n, mut errs) = (0, vec![]);
    for op in j.ops.iter().rev() {
        if !job.step(1) {
            break;
        }
        let r = if op.copied { std::fs::remove_file(&op.to) } else if op.from.exists() { move_file(&op.to, &unique(&op.from)) } else { move_file(&op.to, &op.from) };
        match r {
            Ok(()) => n += 1,
            Err(e) => errs.push(format!("{} : {e}", op.to.display())),
        }
    }
    if errs.is_empty() {
        let _ = std::fs::remove_file(path);
    }
    (n, errs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::job::Jobs;
    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("af_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }
    #[test]
    fn sort_dups_undo() {
        let d = tmp("plan");
        std::fs::write(d.join("a.jpg"), b"same").unwrap();
        std::fs::write(d.join("b.jpg"), b"same").unwrap();
        std::fs::write(d.join("c.mp3"), b"zzzzz").unwrap();
        let mut jobs = Jobs::default();
        let dest = d.join("out");
        let dd = dest.clone();
        let dir = d.clone();
        jobs.spawn("t", "t", move |job| {
            let cfg = Config::default();
            let paths = crate::scan::walk(&dir, false, None);
            let facts = crate::scan::scan(&paths, true, job);
            let items = build(&facts, &cfg, &dd, job);
            assert_eq!(items.len(), 3);
            assert_eq!(items.iter().filter(|i| i.origin == Origin::Dup).count(), 1);
            let (jn, errs) = apply(&items, false, job);
            assert!(errs.is_empty(), "{errs:?}");
            assert_eq!(jn.ops.len(), 3);
            assert!(dd.join("_Doublons").exists());
            let (p, jj) = last_journal().unwrap();
            let (n, e) = undo(&p, &jj, job);
            assert_eq!((n, e.len()), (3, 0));
            Ok("ok".into())
        });
        loop {
            std::thread::sleep(std::time::Duration::from_millis(50));
            let r = jobs.poll();
            if let Some((_, r)) = r.into_iter().next() {
                assert_eq!(r, Ok("ok".to_string()));
                break;
            }
        }
        assert!(d.join("a.jpg").exists() && d.join("b.jpg").exists() && d.join("c.mp3").exists());
        let _ = std::fs::remove_dir_all(&d);
    }
}
