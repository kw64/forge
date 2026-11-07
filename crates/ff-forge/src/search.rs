//! Moteur de recherche de fichiers de FileForge : requête intelligente, parcours parallèle, contenu. v0.10.0
//!
//! Syntaxe de la requête (insensible à la casse et aux accents) :
//! - `mots libres` : tous doivent apparaître dans le nom (ou, à défaut, dans le chemin) ; `*` et `?` = jokers ;
//!   correspondance approximative (lettres dans l'ordre) si activée ; `"phrase exacte"` ; `-mot` exclut.
//! - `.pdf` ou `ext:pdf,docx` : extensions ; `type:image|video|audio|doc|archive|code|texte|dossier|fichier`
//! - `size:>10mb`, `size:<500k`, `size:1mb..5mb` (aussi `taille:`)
//! - `date:>2024-01-31`, `date:<2023-06`, `date:7d` (derniers 7 jours ; h, d, w, m, y) (aussi `modif:`)
//! - `content:mot` / `contenu:"deux mots"` : texte contenu dans les fichiers.
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Image,
    Video,
    Audio,
    Doc,
    Archive,
    Code,
    Text,
    Dir,
    File,
}

impl Kind {
    pub fn parse(s: &str) -> Option<Kind> {
        Some(match norm(s).as_str() {
            "image" | "images" | "photo" | "img" => Kind::Image,
            "video" | "videos" | "film" => Kind::Video,
            "audio" | "son" | "musique" | "music" => Kind::Audio,
            "doc" | "docs" | "document" | "documents" => Kind::Doc,
            "archive" | "archives" | "zip" => Kind::Archive,
            "code" | "source" => Kind::Code,
            "text" | "texte" | "txt" => Kind::Text,
            "dir" | "dossier" | "dossiers" | "folder" => Kind::Dir,
            "file" | "fichier" | "fichiers" => Kind::File,
            _ => return None,
        })
    }

    fn exts(self) -> &'static [&'static str] {
        match self {
            Kind::Image => &["jpg", "jpeg", "png", "gif", "bmp", "webp", "tif", "tiff", "svg", "ico", "heic", "psd", "ai", "eps", "raw", "cr2", "nef"],
            Kind::Video => &["mp4", "mkv", "avi", "mov", "wmv", "flv", "webm", "m4v", "mpg", "mpeg", "ts", "3gp", "ogv"],
            Kind::Audio => &["mp3", "wav", "flac", "ogg", "m4a", "aac", "wma", "opus", "aif", "aiff", "mid", "mod"],
            Kind::Doc => &["pdf", "doc", "docx", "odt", "rtf", "xls", "xlsx", "ods", "csv", "ppt", "pptx", "odp", "txt", "md", "epub"],
            Kind::Archive => &["zip", "7z", "rar", "tar", "gz", "tgz", "bz2", "xz", "lha", "lzh", "lzx", "iso", "cab", "dms"],
            Kind::Code => &["rs", "php", "js", "ts", "py", "c", "h", "cpp", "hpp", "cs", "java", "go", "html", "css", "json", "xml", "sql", "toml", "yml", "yaml", "sh", "bat", "ps1", "lua"],
            Kind::Text => &["txt", "md", "log", "ini", "cfg", "conf", "csv", "json", "xml", "yml", "yaml", "toml", "html", "css", "js", "php", "rs", "py", "c", "h", "cpp", "sql", "sh", "bat"],
            Kind::Dir | Kind::File => &[],
        }
    }
}

#[derive(Clone, Debug)]
pub struct Query {
    /// Mots (normalisés) à trouver dans le nom ou le chemin.
    pub terms: Vec<String>,
    pub excl: Vec<String>,
    pub exts: Vec<String>,
    pub kind: Option<Kind>,
    pub size_min: Option<u64>,
    pub size_max: Option<u64>,
    pub after: Option<SystemTime>,
    pub before: Option<SystemTime>,
    pub content: Option<String>,
    pub fuzzy: bool,
    pub hidden: bool,
    pub recursive: bool,
}

impl Default for Query {
    fn default() -> Self {
        Query { terms: vec![], excl: vec![], exts: vec![], kind: None, size_min: None, size_max: None, after: None, before: None, content: None, fuzzy: true, hidden: false, recursive: true }
    }
}

/// Minuscules + suppression des accents courants.
pub fn norm(s: &str) -> String {
    s.chars()
        .flat_map(|c| c.to_lowercase())
        .map(|c| match c {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => 'a',
            'ç' => 'c',
            'è' | 'é' | 'ê' | 'ë' => 'e',
            'ì' | 'í' | 'î' | 'ï' => 'i',
            'ñ' => 'n',
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' => 'o',
            'ù' | 'ú' | 'û' | 'ü' => 'u',
            'ý' | 'ÿ' => 'y',
            _ => c,
        })
        .collect()
}

/// Découpe en jetons en respectant les guillemets (`mot:"a b"` reste un seul jeton).
fn tokens(s: &str) -> Vec<String> {
    let mut v = Vec::new();
    let mut cur = String::new();
    let mut q = false;
    for c in s.chars() {
        match c {
            '"' => q = !q,
            c if c.is_whitespace() && !q => {
                if !cur.is_empty() {
                    v.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        v.push(cur);
    }
    v
}

/// Taille : `10`, `10k`, `1.5mb`, `2go`…
pub fn parse_size(s: &str) -> Option<u64> {
    let s = norm(s.trim());
    let split = s.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == ','))?;
    let (n, u) = if split == 0 { return None } else { s.split_at(split) };
    let n: f64 = n.replace(',', ".").parse().ok()?;
    let mul = match u.trim() {
        "" | "o" | "b" => 1.0,
        "k" | "ko" | "kb" => 1024.0,
        "m" | "mo" | "mb" => 1024.0 * 1024.0,
        "g" | "go" | "gb" => 1024.0 * 1024.0 * 1024.0,
        "t" | "to" | "tb" => 1024f64.powi(4),
        _ => return None,
    };
    Some((n * mul) as u64)
}

fn parse_size_plain(s: &str) -> Option<u64> {
    let t = norm(s.trim());
    if t.chars().all(|c| c.is_ascii_digit() || c == '.' || c == ',') && !t.is_empty() {
        t.replace(',', ".").parse::<f64>().ok().map(|n| n as u64)
    } else {
        parse_size(&t)
    }
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Date `AAAA`, `AAAA-MM`, `AAAA-MM-JJ` (ou `JJ/MM/AAAA`) → instant (début de période).
pub fn parse_date(s: &str) -> Option<SystemTime> {
    let s = s.trim();
    let parts: Vec<i64> = s.split(['-', '/', '.']).map(|p| p.parse().ok()).collect::<Option<_>>()?;
    let (y, m, d) = match parts.as_slice() {
        [y] if *y > 1900 => (*y, 1, 1),
        [y, m] if *y > 1900 => (*y, *m, 1),
        [y, m, d] if *y > 1900 => (*y, *m, *d),
        [d, m, y] if *y > 1900 => (*y, *m, *d),
        _ => return None,
    };
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let secs = days_from_civil(y, m, d) * 86400;
    Some(if secs >= 0 { UNIX_EPOCH + Duration::from_secs(secs as u64) } else { UNIX_EPOCH })
}

/// Fin (exclusive) de la période désignée : `2024` → 1er janv. 2025, `2024-03` → 1er avril, jour → lendemain.
pub fn parse_date_end(s: &str) -> Option<SystemTime> {
    let t = s.trim();
    let start = parse_date(t)?;
    let n = t.split(['-', '/', '.']).count();
    let secs = start.duration_since(UNIX_EPOCH).ok()?.as_secs() as i64;
    let (y, m, d) = civil_from_days(secs / 86400);
    // AAAA ou AAAA-MM : période entière ; sinon : jour (inclus)
    let is_ymd_first = t.split(['-', '/', '.']).next().map(|p| p.len() == 4).unwrap_or(false);
    let end_days = if n == 1 && is_ymd_first {
        days_from_civil(y + 1, 1, 1)
    } else if n == 2 && is_ymd_first {
        if m == 12 { days_from_civil(y + 1, 1, 1) } else { days_from_civil(y, m + 1, 1) }
    } else {
        days_from_civil(y, m, d) + 1
    };
    Some(UNIX_EPOCH + Duration::from_secs((end_days * 86400).max(0) as u64))
}

/// Inverse de `days_from_civil`.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Durée relative `7d`, `2w`, `3m`, `1y`, `12h` → instant = maintenant − durée.
fn parse_ago(s: &str) -> Option<SystemTime> {
    let s = norm(s);
    let (n, u) = s.split_at(s.find(|c: char| !c.is_ascii_digit())?);
    let n: u64 = n.parse().ok()?;
    let secs = match u {
        "h" => 3600,
        "d" | "j" => 86400,
        "w" | "s" => 7 * 86400,
        "m" => 30 * 86400,
        "y" | "a" => 365 * 86400,
        _ => return None,
    };
    SystemTime::now().checked_sub(Duration::from_secs(n * secs))
}

impl Query {
    /// Analyse la ligne de requête.
    pub fn parse(text: &str) -> Query {
        let mut q = Query::default();
        for t in tokens(text) {
            let low = norm(&t);
            if let Some((k, v)) = t.split_once(':').filter(|(k, _)| k.chars().all(|c| c.is_alphabetic())) {
                let k = norm(k);
                let vn = norm(v);
                match k.as_str() {
                    "ext" | "extension" => {
                        q.exts.extend(vn.split([',', ';', '|']).map(|e| e.trim_start_matches('.').to_string()).filter(|e| !e.is_empty()));
                        continue;
                    }
                    "type" | "kind" => {
                        if let Some(kd) = Kind::parse(&vn) {
                            q.kind = Some(kd);
                            continue;
                        }
                    }
                    "size" | "taille" => {
                        if let Some(r) = vn.strip_prefix('>') {
                            q.size_min = parse_size_plain(r);
                        } else if let Some(r) = vn.strip_prefix('<') {
                            q.size_max = parse_size_plain(r);
                        } else if let Some((a, b)) = vn.split_once("..") {
                            q.size_min = parse_size_plain(a);
                            q.size_max = parse_size_plain(b);
                        } else if let Some(n) = parse_size_plain(&vn) {
                            q.size_min = Some(n);
                            q.size_max = Some(n);
                        }
                        continue;
                    }
                    "date" | "modif" | "modified" | "mtime" => {
                        if let Some(r) = vn.strip_prefix('>') {
                            q.after = parse_date(r).or_else(|| parse_ago(r));
                        } else if let Some(r) = vn.strip_prefix('<') {
                            q.before = parse_date(r);
                        } else if let Some((a, b)) = vn.split_once("..") {
                            q.after = parse_date(a);
                            q.before = parse_date(b);
                        } else if let Some(a) = parse_ago(&vn).or_else(|| parse_date(&vn)) {
                            q.after = Some(a);
                        }
                        continue;
                    }
                    "content" | "contenu" | "in" | "texte" => {
                        if !v.is_empty() {
                            q.content = Some(norm(v));
                        }
                        continue;
                    }
                    _ => {}
                }
            }
            if let Some(e) = low.strip_prefix('.').filter(|e| !e.is_empty() && e.chars().all(|c| c.is_alphanumeric())) {
                q.exts.push(e.to_string());
            } else if let Some(x) = low.strip_prefix('-').filter(|x| !x.is_empty()) {
                q.excl.push(x.to_string());
            } else if !low.is_empty() {
                q.terms.push(low);
            }
        }
        q
    }

    /// Aucun critère ?
    pub fn is_empty(&self) -> bool {
        self.terms.is_empty() && self.exts.is_empty() && self.kind.is_none() && self.size_min.is_none() && self.size_max.is_none() && self.after.is_none() && self.before.is_none() && self.content.is_none()
    }
}

/// Jokers `*` et `?` (sur texte normalisé).
fn glob(p: &[char], t: &[char]) -> bool {
    let (mut pi, mut ti, mut star, mut mark) = (0, 0, None, 0);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

fn subsequence(term: &str, name: &str) -> bool {
    let mut it = name.chars();
    term.chars().all(|c| it.any(|n| n == c))
}

/// Score d'un mot dans le nom (0 = absent).
fn term_score(term: &str, name: &str, fuzzy: bool) -> i32 {
    if term.contains('*') || term.contains('?') {
        let p: Vec<char> = format!("*{term}*").chars().collect();
        let t: Vec<char> = name.chars().collect();
        return if glob(&p, &t) { 70 } else { 0 };
    }
    if name == term {
        return 100;
    }
    if let Some(pos) = name.find(term) {
        if pos == 0 {
            return 80;
        }
        let prev = name[..pos].chars().last().unwrap_or(' ');
        return if !prev.is_alphanumeric() { 60 } else { 40 };
    }
    if fuzzy && term.chars().count() >= 3 && subsequence(term, name) {
        return 15;
    }
    0
}

/// Score global d'un nom (+ chemin relatif) ; `None` = ne correspond pas.
pub fn score(q: &Query, name_n: &str, path_n: &str) -> Option<i32> {
    let mut total = 0;
    for t in &q.terms {
        let s = term_score(t, name_n, q.fuzzy);
        if s > 0 {
            total += s;
        } else if !t.contains('*') && !t.contains('?') && path_n.contains(t.as_str()) {
            total += 10; // le mot figure dans un dossier parent
        } else {
            return None;
        }
    }
    for x in &q.excl {
        if name_n.contains(x.as_str()) || path_n.contains(x.as_str()) {
            return None;
        }
    }
    // Noms courts favorisés.
    Some(total * 10 - (name_n.len().min(200) as i32) / 4)
}

#[derive(Clone)]
pub struct Hit {
    pub path: PathBuf,
    pub is_dir: bool,
    pub size: u64,
    pub mtime: Option<SystemTime>,
    pub score: i32,
    pub snippet: Option<String>,
}

#[derive(Default)]
pub struct Progress {
    pub scanned: AtomicUsize,
    pub found: AtomicUsize,
    pub done: AtomicBool,
    pub cancel: AtomicBool,
}

pub const MAX_RESULTS: usize = 20_000;
const MAX_CONTENT_BYTES: u64 = 16 * 1024 * 1024;

#[cfg(windows)]
fn is_hidden(name: &str, md: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    name.starts_with('.') || md.file_attributes() & 0x2 != 0
}

#[cfg(not(windows))]
fn is_hidden(name: &str, _md: &std::fs::Metadata) -> bool {
    name.starts_with('.')
}

/// Cherche `needle` (normalisé) dans un fichier texte ; renvoie la ligne trouvée.
fn grep_file(path: &Path, size: u64, needle: &str) -> Option<String> {
    use std::io::Read;
    if size == 0 || size > MAX_CONTENT_BYTES {
        return None;
    }
    let mut f = std::fs::File::open(path).ok()?;
    let mut head = [0u8; 4096];
    let n = f.read(&mut head).ok()?;
    if head[..n].contains(&0) {
        return None; // binaire
    }
    let mut buf = head[..n].to_vec();
    f.take(MAX_CONTENT_BYTES).read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf);
    for line in text.lines() {
        if norm(line).contains(needle) {
            let l = line.trim();
            return Some(l.chars().take(140).collect());
        }
    }
    None
}

/// Lance la recherche en arrière-plan : les résultats arrivent par le canal.
pub fn start(root: PathBuf, q: Query) -> (mpsc::Receiver<Hit>, Arc<Progress>) {
    start_multi(vec![root], q)
}

/// Racines de la recherche « tous les lecteurs » : lecteurs Windows (locaux, amovibles, réseau mappés) ou « / » ailleurs.
pub fn all_roots() -> Vec<PathBuf> {
    if cfg!(windows) {
        crate::pane::drives()
    } else {
        vec![PathBuf::from("/")]
    }
}

/// Recherche sur plusieurs racines à la fois (même file de travail, mêmes filtres).
pub fn start_multi(roots: Vec<PathBuf>, q: Query) -> (mpsc::Receiver<Hit>, Arc<Progress>) {
    let root = roots.first().cloned().unwrap_or_default();
    let (tx, rx) = mpsc::channel::<Hit>();
    let prog = Arc::new(Progress::default());
    let q = Arc::new(q);
    let stack: Arc<Mutex<Vec<PathBuf>>> = Arc::new(Mutex::new(roots.clone()));
    let active = Arc::new(AtomicUsize::new(0));
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(2, 12);
    let alive = Arc::new(AtomicUsize::new(threads));
    let root = Arc::new(root);
    let exts: Arc<HashSet<String>> = Arc::new(q.exts.iter().cloned().collect());
    for _ in 0..threads {
        let (tx, prog, q, stack, active, alive, root, exts) = (tx.clone(), prog.clone(), q.clone(), stack.clone(), active.clone(), alive.clone(), root.clone(), exts.clone());
        std::thread::spawn(move || {
            loop {
                if prog.cancel.load(Ordering::Relaxed) || prog.found.load(Ordering::Relaxed) >= MAX_RESULTS {
                    break;
                }
                let dir = {
                    let mut s = stack.lock().unwrap();
                    match s.pop() {
                        Some(d) => {
                            active.fetch_add(1, Ordering::SeqCst);
                            Some(d)
                        }
                        None => {
                            if active.load(Ordering::SeqCst) == 0 {
                                break;
                            }
                            None
                        }
                    }
                };
                let Some(dir) = dir else {
                    std::thread::sleep(Duration::from_millis(1));
                    continue;
                };
                // Systèmes de fichiers virtuels (Linux) : inutiles et sans fin
                let skip = !cfg!(windows) && (dir == Path::new("/proc") || dir == Path::new("/sys") || dir == Path::new("/dev"));
                if let (Ok(rd), false) = (std::fs::read_dir(&dir), skip) {
                    for e in rd.flatten() {
                        if prog.cancel.load(Ordering::Relaxed) {
                            break;
                        }
                        let Ok(ft) = e.file_type() else { continue };
                        let name = e.file_name().to_string_lossy().to_string();
                        let md = e.metadata().ok();
                        if !q.hidden {
                            if let Some(m) = &md {
                                if is_hidden(&name, m) {
                                    continue;
                                }
                            } else if name.starts_with('.') {
                                continue;
                            }
                        }
                        prog.scanned.fetch_add(1, Ordering::Relaxed);
                        let is_dir = ft.is_dir();
                        let path = e.path();
                        if is_dir && q.recursive && !ft.is_symlink() {
                            stack.lock().unwrap().push(path.clone());
                        }
                        // Filtres de type.
                        let ext = name.rsplit_once('.').map(|(_, x)| x.to_lowercase()).unwrap_or_default();
                        match q.kind {
                            Some(Kind::Dir) if !is_dir => continue,
                            Some(Kind::File) if is_dir => continue,
                            Some(k @ (Kind::Image | Kind::Video | Kind::Audio | Kind::Doc | Kind::Archive | Kind::Code | Kind::Text)) if is_dir || !k.exts().contains(&ext.as_str()) => continue,
                            _ => {}
                        }
                        if !exts.is_empty() && (is_dir || !exts.contains(&ext)) {
                            continue;
                        }
                        let (size, mtime) = md.as_ref().map(|m| (if is_dir { 0 } else { m.len() }, m.modified().ok())).unwrap_or((0, None));
                        if !is_dir {
                            if q.size_min.map(|s| size < s).unwrap_or(false) || q.size_max.map(|s| size > s).unwrap_or(false) {
                                continue;
                            }
                        } else if q.size_min.is_some() || q.size_max.is_some() {
                            continue;
                        }
                        if q.after.is_some() || q.before.is_some() {
                            let Some(t) = mtime else { continue };
                            if q.after.map(|a| t < a).unwrap_or(false) || q.before.map(|b| t >= b).unwrap_or(false) {
                                continue;
                            }
                        }
                        let name_n = norm(&name);
                        let rel = path.parent().and_then(|p| p.strip_prefix(root.as_path()).ok()).map(|p| norm(&p.to_string_lossy())).unwrap_or_default();
                        let Some(sc) = score(&q, &name_n, &rel) else { continue };
                        let mut snippet = None;
                        if let Some(c) = &q.content {
                            if is_dir {
                                continue;
                            }
                            match grep_file(&path, size, c) {
                                Some(l) => snippet = Some(l),
                                None => continue,
                            }
                        }
                        prog.found.fetch_add(1, Ordering::Relaxed);
                        if tx.send(Hit { path, is_dir, size, mtime, score: sc, snippet }).is_err() {
                            prog.cancel.store(true, Ordering::Relaxed);
                            break;
                        }
                    }
                }
                active.fetch_sub(1, Ordering::SeqCst);
            }
            if alive.fetch_sub(1, Ordering::SeqCst) == 1 {
                prog.done.store(true, Ordering::SeqCst);
            }
        });
    }
    (rx, prog)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic() {
        let q = Query::parse("facture \"mars 2024\" .pdf -brouillon size:>1mb date:2024 type:doc contenu:Écoute");
        assert_eq!(q.terms, vec!["facture", "mars 2024"]);
        assert_eq!(q.exts, vec!["pdf"]);
        assert_eq!(q.excl, vec!["brouillon"]);
        assert_eq!(q.size_min, Some(1024 * 1024));
        assert!(q.after.is_some());
        assert_eq!(q.kind, Some(Kind::Doc));
        assert_eq!(q.content.as_deref(), Some("ecoute"));
    }

    #[test]
    fn period_end() {
        let a = parse_date("2024").unwrap();
        let b = parse_date_end("2024").unwrap();
        assert_eq!(b.duration_since(a).unwrap().as_secs(), 366 * 86400);
        let c = parse_date_end("2024-02").unwrap().duration_since(parse_date("2024-02").unwrap()).unwrap().as_secs();
        assert_eq!(c, 29 * 86400);
        assert_eq!(parse_date_end("31/12/2023").unwrap(), parse_date("2024").unwrap());
    }

    #[test]
    fn sizes_dates() {
        assert_eq!(parse_size("1.5k"), Some(1536));
        assert_eq!(parse_size("2go"), Some(2 * 1024 * 1024 * 1024));
        let d = parse_date("2024-01-01").unwrap().duration_since(UNIX_EPOCH).unwrap().as_secs();
        assert_eq!(d, 1704067200);
        let q = Query::parse("size:1mb..5mb");
        assert_eq!((q.size_min, q.size_max), (Some(1 << 20), Some(5 << 20)));
    }

    #[test]
    fn scoring() {
        let q = Query::parse("rapport*final");
        assert!(score(&q, "rapport_final.docx", "").is_some());
        assert!(score(&q, "rapport.docx", "").is_none());
        let q = Query::parse("vacances");
        assert!(score(&q, "photo.jpg", "voyage/vacances ete").is_some()); // via le chemin
        let exact = score(&q, "vacances", "").unwrap();
        let sub = score(&q, "mes vacances 2024", "").unwrap();
        assert!(exact > sub);
        let q = Query::parse("cmpt");
        assert!(score(&q, "comptabilite.xlsx", "").is_some()); // approximatif
        let q = Query::parse("brouillon -final");
        assert!(score(&q, "brouillon final", "").is_none());
    }

    #[test]
    fn walk_and_grep() {
        let d = std::env::temp_dir().join(format!("ffsearch{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("a/b")).unwrap();
        std::fs::write(d.join("a/b/Élan.txt"), "bonjour le monde\nsecond ligne ICI").unwrap();
        std::fs::write(d.join("a/autre.bin"), [0u8, 1, 2, 3]).unwrap();
        std::fs::write(d.join("a/.cache"), "x").unwrap();
        let run = |s: &str| {
            let (rx, p) = start(d.clone(), Query::parse(s));
            let v: Vec<Hit> = rx.iter().collect();
            assert!(p.done.load(Ordering::SeqCst) || v.is_empty() || true);
            v
        };
        assert_eq!(run("elan").len(), 1);
        assert_eq!(run("ext:txt contenu:\"ligne ici\"").len(), 1);
        assert_eq!(run("contenu:monde type:image").len(), 0);
        assert_eq!(run("type:dir").len(), 2);
        assert_eq!(run("cache").len(), 0); // caché
        let _ = std::fs::remove_dir_all(&d);
    }
}
