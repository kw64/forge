//! Renommage par lots : calcul du plan (pur) puis application sûre en deux phases.
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CaseMode {
    #[default]
    Keep,
    Lower,
    Upper,
    Title,
}

#[derive(Debug, Clone, Default)]
pub struct RenameRule {
    pub find: String,
    pub replace: String,
    pub prefix: String,
    pub suffix: String,
    pub number: bool,
    pub start: u32,
    pub digits: usize,
    pub number_at_end: bool,
    /// Remplace entièrement le nom de base (le nom d'origine est ignoré ; préfixe, suffixe et numéro s'appliquent).
    pub new_base: String,
    /// Numéro collé au nom (« nom001 ») au lieu de « nom_001 ».
    pub no_sep: bool,
    pub case: CaseMode,
}

fn title_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut up = true;
    for c in s.chars() {
        if c.is_alphanumeric() {
            if up {
                out.extend(c.to_uppercase());
            } else {
                out.extend(c.to_lowercase());
            }
            up = false;
        } else {
            out.push(c);
            up = true;
        }
    }
    out
}

/// Nouveau nom (sans extension) pour le fichier d'indice `idx`.
pub fn new_stem(stem: &str, idx: usize, r: &RenameRule) -> String {
    let mut s = if !r.new_base.is_empty() {
        r.new_base.clone()
    } else if r.find.is_empty() {
        stem.to_string()
    } else {
        stem.replace(&r.find, &r.replace)
    };
    s = match r.case {
        CaseMode::Keep => s,
        CaseMode::Lower => s.to_lowercase(),
        CaseMode::Upper => s.to_uppercase(),
        CaseMode::Title => title_case(&s),
    };
    s = format!("{}{}{}", r.prefix, s, r.suffix);
    if r.number {
        let n = format!("{:0width$}", r.start as usize + idx, width = r.digits.max(1));
        let sep = if r.no_sep { "" } else { "_" };
        s = if r.number_at_end { format!("{s}{sep}{n}") } else { format!("{n}{sep}{s}") };
    }
    s
}

#[derive(Debug, Clone)]
pub struct Step {
    pub from: PathBuf,
    pub to: PathBuf,
    /// Raison du refus (nom vide, caractère interdit, collision…).
    pub conflict: Option<String>,
}

fn invalid_name(name: &str) -> Option<&'static str> {
    if name.trim().is_empty() {
        return Some("nom vide");
    }
    if name.chars().any(|c| matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control()) {
        return Some("caractère interdit");
    }
    if name.ends_with('.') || name.ends_with(' ') {
        return Some("nom invalide");
    }
    None
}

/// Calcule le plan de renommage et détecte les conflits (doublons, fichiers existants).
pub fn plan(files: &[PathBuf], r: &RenameRule) -> Vec<Step> {
    let sources: HashSet<&PathBuf> = files.iter().collect();
    let mut steps: Vec<Step> = files
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let stem = f.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let mut name = new_stem(&stem, i, r);
            if let Some(ext) = f.extension() {
                name.push('.');
                name.push_str(&ext.to_string_lossy());
            }
            let to = f.with_file_name(&name);
            let conflict = invalid_name(&name).map(String::from);
            Step { from: f.clone(), to, conflict }
        })
        .collect();
    let mut seen: HashSet<String> = HashSet::new();
    for s in steps.iter_mut() {
        if s.conflict.is_some() {
            continue;
        }
        let key = s.to.to_string_lossy().to_lowercase();
        if !seen.insert(key) {
            s.conflict = Some("doublon dans le lot".into());
        } else if s.to != s.from && s.to.exists() && !sources.contains(&s.to) {
            s.conflict = Some("un fichier porte déjà ce nom".into());
        }
    }
    steps
}

/// Applique les étapes sans conflit via des noms temporaires (gère les permutations).
/// Retourne le nombre de fichiers renommés et les erreurs rencontrées.
pub fn apply(steps: &[Step]) -> (usize, Vec<String>) {
    let todo: Vec<&Step> = steps.iter().filter(|s| s.conflict.is_none() && s.from != s.to).collect();
    let mut errors = Vec::new();
    let mut staged: Vec<(PathBuf, &Step)> = Vec::new();
    for (i, s) in todo.iter().enumerate() {
        let tmp = s.from.with_file_name(format!(".mf_tmp_{}_{i}", std::process::id()));
        match std::fs::rename(&s.from, &tmp) {
            Ok(()) => staged.push((tmp, s)),
            Err(e) => errors.push(format!("{} : {e}", s.from.display())),
        }
    }
    let mut done = 0;
    for (tmp, s) in staged {
        match std::fs::rename(&tmp, &s.to) {
            Ok(()) => done += 1,
            Err(e) => {
                let _ = std::fs::rename(&tmp, &s.from); // restauration
                errors.push(format!("{} : {e}", s.from.display()));
            }
        }
    }
    (done, errors)
}

/// Vrai si le chemin n'est pas un lien ou un dossier.
pub fn is_plain_file(p: &Path) -> bool {
    p.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stems() {
        let r = RenameRule { find: "IMG".into(), replace: "Photo".into(), number: true, start: 1, digits: 3, ..Default::default() };
        assert_eq!(new_stem("IMG_0042", 0, &r), "001_Photo_0042");
        let r = RenameRule { case: CaseMode::Title, suffix: "-ok".into(), ..Default::default() };
        assert_eq!(new_stem("mon FILM de vacances", 0, &r), "Mon Film De Vacances-ok");
    }

    #[test]
    fn plan_detects_conflicts_and_swaps() {
        let dir = std::env::temp_dir().join(format!("mf_ren_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b, c) = (dir.join("a.txt"), dir.join("b.txt"), dir.join("c.txt"));
        for (p, t) in [(&a, "A"), (&b, "B"), (&c, "C")] {
            std::fs::write(p, t).unwrap();
        }
        // tous au même nom -> doublons
        let r = RenameRule { find: "a".into(), replace: "z".into(), ..Default::default() };
        let steps = plan(&[a.clone(), b.clone()], &r);
        assert!(steps[0].conflict.is_none() && steps[1].conflict.is_none());
        // collision avec un fichier existant hors lot
        let r2 = RenameRule { find: "a".into(), replace: "c".into(), ..Default::default() };
        assert!(plan(&[a.clone()], &r2)[0].conflict.is_some());
        // application réelle
        let (n, errs) = apply(&steps);
        assert_eq!((n, errs.len()), (1, 0));
        assert!(dir.join("z.txt").exists() && !a.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn invalid_names() {
        let r = RenameRule { replace: "x".into(), find: "a".into(), ..Default::default() };
        let steps = plan(&[PathBuf::from("/tmp/a.mp4")], &RenameRule { find: "a".into(), replace: "/".into(), ..r });
        assert!(steps[0].conflict.is_some());
    }

    #[test]
    fn new_base_replaces_everything() {
        let r = RenameRule { new_base: "unautrenom".into(), number: true, number_at_end: true, no_sep: true, start: 1, digits: 3, ..Default::default() };
        let files: Vec<PathBuf> = vec!["/m/nomdefichier.mp3".into(), "/m/autre.mp3".into()];
        let st = plan(&files, &r);
        assert_eq!(st[0].to, PathBuf::from("/m/unautrenom001.mp3"));
        assert_eq!(st[1].to, PathBuf::from("/m/unautrenom002.mp3"));
        assert!(st.iter().all(|s| s.conflict.is_none()));
        // sans numéro, deux fichiers donnent le même nom : conflit détecté
        let r2 = RenameRule { new_base: "x".into(), ..Default::default() };
        assert!(plan(&files, &r2)[1].conflict.is_some());
    }
}
