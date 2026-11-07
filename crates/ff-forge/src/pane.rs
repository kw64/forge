//! Un panneau de FileForge : emplacement (disque, lecteurs ou archive), lignes, sélection. v0.6.0
use crate::archive::{self, Archive};
use crate::fsops;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

#[derive(Clone)]
pub struct Row {
    /// Nom affiché.
    pub name: String,
    /// Clé unique : chemin interne (archive) ou nom (disque).
    pub full: String,
    pub is_dir: bool,
    pub size: u64,
    pub mtime: Option<SystemTime>,
    /// Fichier d'archive navigable.
    pub is_arc: bool,
    pub is_parent: bool,
}

#[derive(Clone)]
pub enum Loc {
    Drives,
    Disk(PathBuf),
    Arc { arch: Arc<Archive>, inner: String, back: PathBuf },
}

pub struct Pane {
    pub loc: Loc,
    pub rows: Vec<Row>,
    pub sel: HashSet<String>,
    pub cursor: Option<usize>,
    pub path_edit: String,
    pub err: Option<String>,
    pub scroll_to_cursor: bool,
    /// Libellés en anglais.
    pub en: bool,
    pub hidden: Vec<String>,
    pub custom: Vec<String>,
    /// Tri : 0 nom, 1 date, 2 type (extension), 3 taille ; `sort_desc` : ordre décroissant (dossiers toujours en premier).
    pub sort_key: u8,
    pub sort_desc: bool,
}

pub fn drives() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if cfg!(windows) {
        for c in b'A'..=b'Z' {
            let p = PathBuf::from(format!("{}:\\", c as char));
            if p.exists() {
                v.push(p);
            }
        }
    } else {
        v.push(PathBuf::from("/"));
        if let Some(h) = dirs::home_dir() {
            v.push(h);
        }
        for base in ["/mnt", "/media"] {
            if let Ok(rd) = std::fs::read_dir(base) {
                v.extend(rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()));
            }
        }
    }
    v
}

/// Dossiers personnels proposés (identifiant, libellé FR, libellé EN, chemin).
pub fn standard_places() -> Vec<(&'static str, &'static str, &'static str, Option<PathBuf>)> {
    vec![
        ("home", "Dossier personnel", "Home", dirs::home_dir()),
        ("desktop", "Bureau", "Desktop", dirs::desktop_dir()),
        ("downloads", "Téléchargements", "Downloads", dirs::download_dir()),
        ("documents", "Documents", "Documents", dirs::document_dir()),
        ("pictures", "Images", "Pictures", dirs::picture_dir()),
        ("videos", "Vidéos", "Videos", dirs::video_dir()),
        ("music", "Musique", "Music", dirs::audio_dir()),
    ]
}

/// Dossiers affichés dans « Lecteurs » : standards non masqués et existants, puis dossiers ajoutés.
pub fn places(en: bool, hidden: &[String], custom: &[String]) -> Vec<(String, PathBuf)> {
    let mut v: Vec<(String, PathBuf)> = standard_places()
        .into_iter()
        .filter(|(id, ..)| !hidden.iter().any(|h| h == id))
        .filter_map(|(_, fr, e, p)| p.filter(|p| p.is_dir()).map(|p| (if en { e } else { fr }.to_string(), p)))
        .collect();
    for c in custom {
        let p = PathBuf::from(c);
        if p.is_dir() {
            let n = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| c.clone());
            v.push((n, p));
        }
    }
    v
}

impl Pane {
    pub fn new(p: PathBuf) -> Pane {
        let mut s = Pane { loc: Loc::Disk(p), rows: vec![], sel: HashSet::new(), cursor: None, path_edit: String::new(), err: None, scroll_to_cursor: false, en: false, hidden: vec![], custom: vec![], sort_key: 0, sort_desc: false };
        s.refresh();
        s
    }

    pub fn title(&self) -> String {
        match &self.loc {
            Loc::Drives => "Drives:".into(),
            Loc::Disk(p) => p.display().to_string(),
            Loc::Arc { arch, inner, .. } => format!("{}:/{}", arch.path.display(), inner),
        }
    }

    /// Dossier disque courant (None dans une archive ou la liste des lecteurs).
    pub fn disk_dir(&self) -> Option<&Path> {
        match &self.loc {
            Loc::Disk(p) => Some(p),
            _ => None,
        }
    }

    pub fn refresh(&mut self) {
        self.err = None;
        let mut rows: Vec<Row> = Vec::new();
        match &self.loc {
            Loc::Drives => {
                // Dossiers personnels d'abord (« ★ Bureau »), puis les lecteurs ; `full` porte le chemin.
                for (n, p) in places(self.en, &self.hidden, &self.custom) {
                    rows.push(Row { name: format!("★ {n}"), full: p.display().to_string(), is_dir: true, size: 0, mtime: None, is_arc: false, is_parent: false });
                }
                for d in drives() {
                    let n = d.display().to_string();
                    rows.push(Row { name: n.clone(), full: n, is_dir: true, size: 0, mtime: None, is_arc: false, is_parent: false });
                }
            }
            Loc::Disk(p) => match std::fs::read_dir(p) {
                Ok(rd) => {
                    for e in rd.flatten() {
                        let m = e.metadata().ok();
                        let n = e.file_name().to_string_lossy().into_owned();
                        let is_dir = m.as_ref().map(|m| m.is_dir()).unwrap_or(false);
                        rows.push(Row {
                            is_arc: !is_dir && archive::kind_of(Path::new(&n)).is_some(),
                            name: n.clone(),
                            full: n,
                            is_dir,
                            size: m.as_ref().map(|m| if m.is_dir() { 0 } else { m.len() }).unwrap_or(0),
                            mtime: m.and_then(|m| m.modified().ok()),
                            is_parent: false,
                        });
                    }
                }
                Err(e) => self.err = Some(e.to_string()),
            },
            Loc::Arc { arch, inner, .. } => {
                for e in arch.list(inner) {
                    let n = e.name.rsplit('/').next().unwrap_or(&e.name).to_string();
                    rows.push(Row { is_arc: !e.is_dir && archive::kind_of(Path::new(&n)).is_some(), name: n, full: e.name, is_dir: e.is_dir, size: e.size, mtime: None, is_parent: false });
                }
            }
        }
        if !matches!(self.loc, Loc::Drives) {
            sort_rows(&mut rows, self.sort_key, self.sort_desc);
        }
        if !matches!(self.loc, Loc::Drives) {
            rows.insert(0, Row { name: "..".into(), full: "..".into(), is_dir: true, size: 0, mtime: None, is_arc: false, is_parent: true });
        }
        self.rows = rows;
        self.sel.clear();
        self.cursor = if self.rows.is_empty() { None } else { Some(0) };
        self.path_edit = self.title();
        self.scroll_to_cursor = true;
    }

    /// Retrie la liste affichée sans relire le disque (la sélection et le curseur sont conservés).
    pub fn resort(&mut self) {
        if matches!(self.loc, Loc::Drives) {
            return;
        }
        let cur = self.cursor.and_then(|i| self.rows.get(i)).map(|r| r.full.clone());
        let parent = if self.rows.first().map(|r| r.is_parent).unwrap_or(false) { Some(self.rows.remove(0)) } else { None };
        sort_rows(&mut self.rows, self.sort_key, self.sort_desc);
        if let Some(p) = parent {
            self.rows.insert(0, p);
        }
        self.cursor = cur.and_then(|c| self.rows.iter().position(|r| r.full == c)).or(if self.rows.is_empty() { None } else { Some(0) });
        self.scroll_to_cursor = true;
    }

    pub fn goto(&mut self, loc: Loc) {
        self.loc = loc;
        self.refresh();
    }

    pub fn parent(&mut self) {
        let new = match &self.loc {
            Loc::Drives => return,
            Loc::Disk(p) => match p.parent() {
                Some(pp) => Loc::Disk(pp.to_path_buf()),
                None => Loc::Drives,
            },
            Loc::Arc { arch, inner, back } => {
                if inner.is_empty() {
                    Loc::Disk(back.clone())
                } else {
                    let up = inner.rsplit_once('/').map(|x| x.0).unwrap_or("").to_string();
                    Loc::Arc { arch: arch.clone(), inner: up, back: back.clone() }
                }
            }
        };
        // Revenir en sélectionnant le dossier quitté.
        let leaving = match &self.loc {
            Loc::Disk(p) => p.file_name().map(|n| n.to_string_lossy().into_owned()),
            Loc::Arc { inner, arch, .. } if inner.is_empty() => arch.path.file_name().map(|n| n.to_string_lossy().into_owned()),
            Loc::Arc { inner, .. } => inner.rsplit('/').next().map(|s| s.to_string()),
            _ => None,
        };
        self.goto(new);
        if let Some(l) = leaving {
            if let Some(i) = self.rows.iter().position(|r| r.name == l) {
                self.cursor = Some(i);
            }
        }
    }

    /// Chemin disque d'une ligne (None dans une archive).
    pub fn disk_path(&self, r: &Row) -> Option<PathBuf> {
        match &self.loc {
            Loc::Disk(p) => Some(p.join(&r.name)),
            _ => None,
        }
    }

    /// Ouvre un dossier / une archive ; retourne le chemin du fichier à ouvrir sinon.
    pub fn enter(&mut self, i: usize) -> Option<PathBuf> {
        let r = self.rows.get(i)?.clone();
        if r.is_parent {
            self.parent();
            return None;
        }
        match self.loc.clone() {
            Loc::Drives => self.goto(Loc::Disk(PathBuf::from(&r.full))),
            Loc::Disk(p) => {
                let full = p.join(&r.name);
                if r.is_dir {
                    self.goto(Loc::Disk(full));
                } else if r.is_arc {
                    match Archive::open(&full) {
                        Ok(a) => self.goto(Loc::Arc { arch: Arc::new(a), inner: String::new(), back: p }),
                        Err(e) => self.err = Some(e),
                    }
                } else {
                    return Some(full);
                }
            }
            Loc::Arc { arch, back, .. } => {
                if r.is_dir {
                    self.goto(Loc::Arc { arch, inner: r.full, back });
                } else {
                    return None;
                }
            }
        }
        None
    }

    /// Lignes visées par une opération : la sélection, sinon la ligne du curseur.
    pub fn targets(&self) -> Vec<Row> {
        let s: Vec<Row> = self.rows.iter().filter(|r| !r.is_parent && self.sel.contains(&r.full)).cloned().collect();
        if !s.is_empty() {
            return s;
        }
        self.cursor.and_then(|i| self.rows.get(i)).filter(|r| !r.is_parent).cloned().into_iter().collect()
    }

    pub fn stats(&self) -> (usize, usize, usize, usize, u64, u64) {
        // (fichiers sél., fichiers, dossiers sél., dossiers, octets sél., octets)
        let (mut fs, mut f, mut ds, mut d, mut bs, mut b) = (0, 0, 0, 0, 0, 0);
        for r in self.rows.iter().filter(|r| !r.is_parent) {
            let s = self.sel.contains(&r.full);
            if r.is_dir {
                d += 1;
                if s { ds += 1 }
            } else {
                f += 1;
                b += r.size;
                if s { fs += 1; bs += r.size }
            }
        }
        (fs, f, ds, d, bs, b)
    }

    pub fn space(&self) -> Option<(u64, u64)> {
        let p = match &self.loc {
            Loc::Disk(p) => p.clone(),
            Loc::Arc { arch, .. } => arch.path.clone(),
            Loc::Drives => return None,
        };
        let t = fs2::total_space(&p).ok()?;
        let a = fs2::available_space(&p).ok()?;
        Some((t.saturating_sub(a), t))
    }
}

pub fn human(b: u64) -> String {
    if b >= 1 << 30 { format!("{:.1} G", b as f64 / (1u64 << 30) as f64) } else if b >= 1 << 20 { format!("{} M", b >> 20) } else { fsops::spaced(b) }
}

/// Tri des lignes : dossiers d'abord, puis la clé choisie (0 nom, 1 date, 2 type, 3 taille).
pub fn sort_rows(rows: &mut [Row], key: u8, desc: bool) {
    use std::cmp::Ordering;
    let ext = |r: &Row| Path::new(&r.name).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    rows.sort_by(|a, b| {
        let by_name = || ff_core::media::natural_cmp(&a.name, &b.name);
        let o = match key {
            1 => a.mtime.cmp(&b.mtime).then_with(by_name),
            2 => (!a.is_dir).cmp(&!b.is_dir).then_with(|| ext(a).cmp(&ext(b))).then_with(by_name),
            3 => a.size.cmp(&b.size).then_with(by_name),
            _ => by_name(),
        };
        // Les dossiers restent en tête quel que soit le sens.
        b.is_dir.cmp(&a.is_dir).then(if desc && !(a.is_dir && b.is_dir && key == 3) { o.reverse() } else { o }).then(Ordering::Equal)
    });
}
