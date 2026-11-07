//! Moteur de renommage en lot de FileForge (règles chaînées, aperçu, validation, annulation). v0.11.0
//! Inspiré de ReNamer : Insérer, Supprimer, Retirer, Remplacer, Réorganiser, Extension, Nettoyer, Casse, Numéroter, Date.
use regex::RegexBuilder;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum Scope {
    Name,
    Ext,
    Both,
}

#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
pub enum Place {
    Prefix,
    Suffix,
    /// Position 1-based (depuis la gauche, ou la droite selon l'option de la règle).
    Position(usize),
    After(String),
    Before(String),
    /// Remplace tout le nom (Numéroter).
    Replace,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum Which {
    All,
    First,
    Last,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum DelMode {
    Count,
    Until,
    End,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum ExtMode {
    Lower,
    Upper,
    Set,
    Remove,
    AddIfMissing,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum CaseMode {
    Lower,
    Upper,
    Title,
    Sentence,
    Invert,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum DateSrc {
    Modified,
    Created,
    Now,
}

#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
pub enum Kind {
    Insert { text: String, place: Place, from_right: bool },
    Delete { from: usize, from_right: bool, mode: DelMode, count: usize, until: String },
    Remove { text: String, case: bool, which: Which },
    Replace { find: String, with: String, case: bool, regex: bool, which: Which },
    Rearrange { delims: String, pattern: String },
    Extension { mode: ExtMode, text: String },
    Strip { letters: bool, digits: bool, symbols: bool, brackets: bool, accents: bool, spaces: bool, custom: String },
    Case { mode: CaseMode, exceptions: String },
    Serialize { start: i64, step: i64, pad: usize, place: Place, sep: String, per_folder: bool },
    Date { format: String, place: Place, source: DateSrc },
}

#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
pub struct RuleItem {
    pub enabled: bool,
    pub scope: Scope,
    pub kind: Kind,
}

impl RuleItem {
    pub fn new(kind: Kind) -> RuleItem {
        RuleItem { enabled: true, scope: Scope::Name, kind }
    }
}

pub const TYPES: &[(&str, &str)] = &[("Insérer", "Insert"), ("Supprimer", "Delete"), ("Retirer", "Remove"), ("Remplacer", "Replace"), ("Réorganiser", "Rearrange"), ("Extension", "Extension"), ("Nettoyer", "Strip"), ("Casse", "Case"), ("Numéroter", "Serialize"), ("Date", "Date")];

/// Règle par défaut du type `i` (index dans `TYPES`).
pub fn default_rule(i: usize) -> RuleItem {
    let kind = match i {
        0 => Kind::Insert { text: String::new(), place: Place::Prefix, from_right: false },
        1 => Kind::Delete { from: 1, from_right: false, mode: DelMode::Count, count: 1, until: String::new() },
        2 => Kind::Remove { text: String::new(), case: false, which: Which::All },
        3 => Kind::Replace { find: String::new(), with: String::new(), case: false, regex: false, which: Which::All },
        4 => Kind::Rearrange { delims: " -_".into(), pattern: "$2 $1".into() },
        5 => Kind::Extension { mode: ExtMode::Lower, text: String::new() },
        6 => Kind::Strip { letters: false, digits: false, symbols: false, brackets: false, accents: false, spaces: true, custom: String::new() },
        7 => Kind::Case { mode: CaseMode::Title, exceptions: "de, la, le, les, du, des, et, a, the, of, and".into() },
        8 => Kind::Serialize { start: 1, step: 1, pad: 3, place: Place::Suffix, sep: " ".into(), per_folder: false },
        _ => Kind::Date { format: "yyyy-mm-dd".into(), place: Place::Prefix, source: DateSrc::Modified },
    };
    RuleItem::new(kind)
}

#[allow(dead_code)]
pub fn type_index(k: &Kind) -> usize {
    match k {
        Kind::Insert { .. } => 0,
        Kind::Delete { .. } => 1,
        Kind::Remove { .. } => 2,
        Kind::Replace { .. } => 3,
        Kind::Rearrange { .. } => 4,
        Kind::Extension { .. } => 5,
        Kind::Strip { .. } => 6,
        Kind::Case { .. } => 7,
        Kind::Serialize { .. } => 8,
        Kind::Date { .. } => 9,
    }
}

/// Résumé lisible d'une règle (colonne « Règle / Instruction »).
pub fn describe(r: &RuleItem, en: bool) -> String {
    let t = |fr: &str, e: &str| if en { e.to_string() } else { fr.to_string() };
    let place = |p: &Place| match p {
        Place::Prefix => t("au début", "at start"),
        Place::Suffix => t("à la fin", "at end"),
        Place::Position(n) => format!("{} {n}", t("à la position", "at position")),
        Place::After(s) => format!("{} « {s} »", t("après", "after")),
        Place::Before(s) => format!("{} « {s} »", t("avant", "before")),
        Place::Replace => t("à la place du nom", "replacing the name"),
    };
    match &r.kind {
        Kind::Insert { text, place: p, .. } => format!("{} « {text} » {}", t("Insérer", "Insert"), place(p)),
        Kind::Delete { from, mode, count, until, from_right } => {
            let side = if *from_right { t("depuis la droite", "from the right") } else { t("depuis la gauche", "from the left") };
            match mode {
                DelMode::Count => format!("{} {count} {} {side} {from}", t("Supprimer", "Delete"), t("car. à partir de", "chars from")),
                DelMode::Until => format!("{} {side} {from} {} « {until} »", t("Supprimer", "Delete"), t("jusqu'à", "until")),
                DelMode::End => format!("{} {side} {from} {}", t("Supprimer", "Delete"), t("jusqu'à la fin", "to the end")),
            }
        }
        Kind::Remove { text, .. } => format!("{} « {text} »", t("Retirer", "Remove")),
        Kind::Replace { find, with, regex, .. } => format!("{} « {find} » → « {with} »{}", t("Remplacer", "Replace"), if *regex { " (regex)" } else { "" }),
        Kind::Rearrange { delims, pattern } => format!("{} « {delims} » → {pattern}", t("Réorganiser", "Rearrange")),
        Kind::Extension { mode, text } => match mode {
            ExtMode::Lower => t("Extension en minuscules", "Lowercase extension"),
            ExtMode::Upper => t("Extension en MAJUSCULES", "Uppercase extension"),
            ExtMode::Set => format!("{} « {text} »", t("Extension fixée à", "Set extension to")),
            ExtMode::Remove => t("Supprimer l'extension", "Remove extension"),
            ExtMode::AddIfMissing => format!("{} « {text} »", t("Ajouter si absente", "Add if missing")),
        },
        Kind::Strip { .. } => t("Nettoyer (caractères, accents, espaces…)", "Strip (characters, accents, spaces…)"),
        Kind::Case { mode, .. } => match mode {
            CaseMode::Lower => t("Casse : minuscules", "Case: lower"),
            CaseMode::Upper => t("Casse : MAJUSCULES", "Case: UPPER"),
            CaseMode::Title => t("Casse : Titre", "Case: Title"),
            CaseMode::Sentence => t("Casse : Phrase", "Case: Sentence"),
            CaseMode::Invert => t("Casse : inverser", "Case: invert"),
        },
        Kind::Serialize { start, step, pad, place: p, .. } => format!("{} {start}, +{step}, {pad} {} {}", t("Numéroter dès", "Number from"), t("chiffres", "digits"), place(p)),
        Kind::Date { format, place: p, source } => format!("{} « {format} » ({}) {}", t("Date", "Date"), match source { DateSrc::Modified => t("modif.", "modified"), DateSrc::Created => t("création", "created"), DateSrc::Now => t("maintenant", "now") }, place(p)),
    }
}

// ---- Informations fichier ---------------------------------------------------

#[derive(Clone)]
pub struct FileInfo {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub mtime: Option<SystemTime>,
    pub ctime: Option<SystemTime>,
    pub size: u64,
}

impl FileInfo {
    pub fn from_path(p: &Path) -> Option<FileInfo> {
        let md = std::fs::metadata(p).ok()?;
        Some(FileInfo { path: p.to_path_buf(), name: p.file_name()?.to_string_lossy().to_string(), is_dir: md.is_dir(), mtime: md.modified().ok(), ctime: md.created().ok(), size: if md.is_dir() { 0 } else { md.len() } })
    }
}

/// Nom scindé en (radical, extension) ; un fichier « .bashrc » n'a pas d'extension. Un dossier non plus.
pub fn split_name(name: &str, is_dir: bool) -> (String, String) {
    if is_dir {
        return (name.to_string(), String::new());
    }
    match name.rfind('.') {
        Some(i) if i > 0 => (name[..i].to_string(), name[i + 1..].to_string()),
        _ => (name.to_string(), String::new()),
    }
}

fn join_name(stem: &str, ext: &str) -> String {
    if ext.is_empty() {
        stem.to_string()
    } else {
        format!("{stem}.{ext}")
    }
}

// ---- Utilitaires texte ------------------------------------------------------

fn eq_char(a: char, b: char, case: bool) -> bool {
    if case {
        a == b
    } else {
        a == b || a.to_lowercase().eq(b.to_lowercase())
    }
}

/// Positions (en caractères) des occurrences non chevauchantes.
fn find_all(hay: &[char], needle: &[char], case: bool) -> Vec<usize> {
    let mut v = Vec::new();
    if needle.is_empty() || needle.len() > hay.len() {
        return v;
    }
    let mut i = 0;
    while i + needle.len() <= hay.len() {
        if needle.iter().enumerate().all(|(j, n)| eq_char(hay[i + j], *n, case)) {
            v.push(i);
            i += needle.len();
        } else {
            i += 1;
        }
    }
    v
}

fn strip_accents(c: char) -> char {
    match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => 'a',
        'À' | 'Á' | 'Â' | 'Ã' | 'Ä' | 'Å' => 'A',
        'ç' => 'c',
        'Ç' => 'C',
        'è' | 'é' | 'ê' | 'ë' => 'e',
        'È' | 'É' | 'Ê' | 'Ë' => 'E',
        'ì' | 'í' | 'î' | 'ï' => 'i',
        'Ì' | 'Í' | 'Î' | 'Ï' => 'I',
        'ñ' => 'n',
        'Ñ' => 'N',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' => 'o',
        'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' => 'O',
        'ù' | 'ú' | 'û' | 'ü' => 'u',
        'Ù' | 'Ú' | 'Û' | 'Ü' => 'U',
        'ý' | 'ÿ' => 'y',
        'Ý' => 'Y',
        _ => c,
    }
}

/// Insère `ins` dans `s` selon `place`.
fn insert_at(s: &str, ins: &str, place: &Place, from_right: bool) -> String {
    let chars: Vec<char> = s.chars().collect();
    let at = match place {
        Place::Prefix => 0,
        Place::Suffix => chars.len(),
        Place::Position(n) => {
            let n = (*n).max(1) - 1;
            if from_right {
                chars.len().saturating_sub(n)
            } else {
                n.min(chars.len())
            }
        }
        Place::After(t) | Place::Before(t) => {
            let needle: Vec<char> = t.chars().collect();
            match find_all(&chars, &needle, false).first() {
                Some(&p) => {
                    if matches!(place, Place::After(_)) {
                        p + needle.len()
                    } else {
                        p
                    }
                }
                None => return s.to_string(),
            }
        }
        Place::Replace => return ins.to_string(),
    };
    let mut out: String = chars[..at].iter().collect();
    out.push_str(ins);
    out.extend(&chars[at..]);
    out
}

pub fn fmt_date(t: SystemTime, f: &str) -> String {
    let s = t.duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let (days, rem) = (s.div_euclid(86400), s.rem_euclid(86400));
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    f.replace("yyyy", &format!("{y:04}"))
        .replace("yy", &format!("{:02}", y % 100))
        .replace("mm", &format!("{m:02}"))
        .replace("dd", &format!("{d:02}"))
        .replace("hh", &format!("{:02}", rem / 3600))
        .replace("nn", &format!("{:02}", rem % 3600 / 60))
        .replace("ss", &format!("{:02}", rem % 60))
}

fn title_case(s: &str, exceptions: &str) -> String {
    let exc: Vec<String> = exceptions.split([',', ';', ' ']).map(|x| x.trim().to_lowercase()).filter(|x| !x.is_empty()).collect();
    let mut out = String::new();
    let mut word = String::new();
    let mut first = true;
    let flush = |word: &mut String, out: &mut String, first: &mut bool| {
        if word.is_empty() {
            return;
        }
        let low = word.to_lowercase();
        if !*first && exc.contains(&low) {
            out.push_str(&low);
        } else {
            let mut c = low.chars();
            if let Some(f) = c.next() {
                out.extend(f.to_uppercase());
                out.push_str(c.as_str());
            }
        }
        *first = false;
        word.clear();
    };
    for c in s.chars() {
        if c.is_alphanumeric() || c == '\'' {
            word.push(c);
        } else {
            flush(&mut word, &mut out, &mut first);
            out.push(c);
        }
    }
    flush(&mut word, &mut out, &mut first);
    out
}

// ---- Application des règles -------------------------------------------------

pub struct Ctx<'a> {
    pub info: &'a FileInfo,
    /// Rang du fichier parmi les fichiers actifs.
    pub index: usize,
    /// Rang dans son dossier.
    pub index_in_folder: usize,
}

/// Applique une règle à une chaîne (radical, extension ou nom complet selon la portée).
fn apply_text(k: &Kind, s: &str, ctx: &Ctx) -> Result<String, String> {
    Ok(match k {
        Kind::Extension { .. } => s.to_string(), // traité dans apply_rules
        Kind::Insert { text, place, from_right } => insert_at(s, text, place, *from_right),
        Kind::Delete { from, from_right, mode, count, until } => {
            let chars: Vec<char> = s.chars().collect();
            let n = chars.len();
            let p = (*from).max(1) - 1;
            let (a, b) = if !*from_right {
                let a = p.min(n);
                let b = match mode {
                    DelMode::Count => (a + count).min(n),
                    DelMode::End => n,
                    DelMode::Until => {
                        let needle: Vec<char> = until.chars().collect();
                        find_all(&chars[a..], &needle, false).first().map(|i| a + i).unwrap_or(n)
                    }
                };
                (a, b)
            } else {
                let b = n.saturating_sub(p);
                let a = match mode {
                    DelMode::Count => b.saturating_sub(*count),
                    DelMode::End => 0,
                    DelMode::Until => {
                        let needle: Vec<char> = until.chars().collect();
                        find_all(&chars[..b], &needle, false).last().map(|i| i + needle.len()).unwrap_or(0)
                    }
                };
                (a, b)
            };
            chars[..a].iter().chain(chars[b.max(a)..].iter()).collect()
        }
        Kind::Remove { text, case, which } => {
            let chars: Vec<char> = s.chars().collect();
            let needle: Vec<char> = text.chars().collect();
            let pos = find_all(&chars, &needle, *case);
            let sel: Vec<usize> = match which {
                Which::All => pos,
                Which::First => pos.into_iter().take(1).collect(),
                Which::Last => pos.into_iter().last().into_iter().collect(),
            };
            let mut out = String::new();
            let mut i = 0;
            while i < chars.len() {
                if sel.contains(&i) {
                    i += needle.len();
                } else {
                    out.push(chars[i]);
                    i += 1;
                }
            }
            out
        }
        Kind::Replace { find, with, case, regex, which } => {
            if find.is_empty() {
                return Ok(s.to_string());
            }
            let pat = if *regex { find.clone() } else { regex::escape(find) };
            let re = RegexBuilder::new(&pat).case_insensitive(!*case).build().map_err(|e| format!("regex : {}", e.to_string().lines().last().unwrap_or("")))?;
            // « $2_$1 » : le moteur lirait « $2_ » comme un nom de groupe ; on force « ${2}_${1} ».
            let with = &{
                let mut o = String::new();
                let c: Vec<char> = with.chars().collect();
                let mut i = 0;
                while i < c.len() {
                    if c[i] == '$' && i + 1 < c.len() && c[i + 1].is_ascii_digit() {
                        let mut j = i + 1;
                        while j < c.len() && c[j].is_ascii_digit() {
                            j += 1;
                        }
                        o.push_str(&format!("${{{}}}", c[i + 1..j].iter().collect::<String>()));
                        i = j;
                    } else {
                        o.push(c[i]);
                        i += 1;
                    }
                }
                o
            };
            let caps: Vec<regex::Captures> = re.captures_iter(s).collect();
            let n = caps.len();
            let mut out = String::new();
            let mut last = 0;
            for (i, c) in caps.iter().enumerate() {
                let pick = match which {
                    Which::All => true,
                    Which::First => i == 0,
                    Which::Last => i + 1 == n,
                };
                if pick {
                    let m = c.get(0).unwrap();
                    out.push_str(&s[last..m.start()]);
                    if *regex {
                        c.expand(with, &mut out);
                    } else {
                        out.push_str(with);
                    }
                    last = m.end();
                }
            }
            out.push_str(&s[last..]);
            out
        }
        Kind::Rearrange { delims, pattern } => {
            let parts: Vec<&str> = s.split(|c| delims.contains(c)).filter(|p| !p.is_empty()).collect();
            let pc: Vec<char> = pattern.chars().collect();
            let mut out = String::new();
            let mut i = 0;
            while i < pc.len() {
                if pc[i] == '$' {
                    let mut j = i + 1;
                    let neg = j < pc.len() && pc[j] == '-';
                    if neg {
                        j += 1;
                    }
                    let st = j;
                    while j < pc.len() && pc[j].is_ascii_digit() {
                        j += 1;
                    }
                    if j > st {
                        let n: usize = pc[st..j].iter().collect::<String>().parse().unwrap_or(0);
                        let part = if n == 0 {
                            Some(&s[..])
                        } else if neg {
                            parts.len().checked_sub(n).and_then(|k| parts.get(k).copied())
                        } else {
                            parts.get(n - 1).copied()
                        };
                        out.push_str(part.unwrap_or(""));
                        i = j;
                        continue;
                    }
                }
                out.push(pc[i]);
                i += 1;
            }
            out
        }
        Kind::Strip { letters, digits, symbols, brackets, accents, spaces, custom } => {
            let mut t = s.to_string();
            if *brackets {
                let re = regex::Regex::new(r"\([^)]*\)|\[[^\]]*\]|\{[^}]*\}").unwrap();
                t = re.replace_all(&t, "").to_string();
            }
            let mut o: String = t
                .chars()
                .filter(|c| !((*letters && c.is_alphabetic()) || (*digits && c.is_ascii_digit()) || (*symbols && !c.is_alphanumeric() && !c.is_whitespace()) || custom.contains(*c)))
                .collect();
            if *accents {
                o = o.chars().map(strip_accents).collect();
            }
            if *spaces {
                o = o.split_whitespace().collect::<Vec<_>>().join(" ");
            }
            o
        }
        Kind::Case { mode, exceptions } => match mode {
            CaseMode::Lower => s.to_lowercase(),
            CaseMode::Upper => s.to_uppercase(),
            CaseMode::Title => title_case(s, exceptions),
            CaseMode::Sentence => {
                let low = s.to_lowercase();
                let mut c = low.chars();
                c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
            }
            CaseMode::Invert => s.chars().map(|c| if c.is_uppercase() { c.to_lowercase().next().unwrap_or(c) } else { c.to_uppercase().next().unwrap_or(c) }).collect(),
        },
        Kind::Serialize { start, step, pad, place, sep, per_folder } => {
            let idx = if *per_folder { ctx.index_in_folder } else { ctx.index } as i64;
            let v = start + step * idx;
            let num = if v < 0 { format!("-{:0>w$}", -v, w = *pad) } else { format!("{:0>w$}", v, w = *pad) };
            match place {
                Place::Replace => num,
                Place::Prefix => format!("{num}{sep}{s}"),
                Place::Suffix => format!("{s}{sep}{num}"),
                p => insert_at(s, &num, p, false),
            }
        }
        Kind::Date { format, place, source } => {
            let t = match source {
                DateSrc::Modified => ctx.info.mtime,
                DateSrc::Created => ctx.info.ctime.or(ctx.info.mtime),
                DateSrc::Now => Some(SystemTime::now()),
            };
            match t {
                Some(t) => insert_at(s, &fmt_date(t, format), place, false),
                None => s.to_string(),
            }
        }
    })
}

/// Applique la chaîne de règles à un nom.
pub fn apply_rules(rules: &[RuleItem], ctx: &Ctx) -> Result<String, String> {
    let (mut stem, mut ext) = split_name(&ctx.info.name, ctx.info.is_dir);
    for r in rules.iter().filter(|r| r.enabled) {
        // Extension : agit directement sur l'extension, quelle que soit la portée.
        if let Kind::Extension { mode, text } = &r.kind {
            let t = text.trim_start_matches('.');
            match mode {
                ExtMode::Lower => ext = ext.to_lowercase(),
                ExtMode::Upper => ext = ext.to_uppercase(),
                ExtMode::Set => ext = t.to_string(),
                ExtMode::Remove => ext.clear(),
                ExtMode::AddIfMissing => {
                    if ext.is_empty() {
                        ext = t.to_string()
                    }
                }
            }
            continue;
        }
        if ctx.info.is_dir && r.scope == Scope::Ext {
            continue;
        }
        match r.scope {
            Scope::Name => stem = apply_text(&r.kind, &stem, ctx)?,
            Scope::Ext => ext = apply_text(&r.kind, &ext, ctx)?,
            Scope::Both => {
                let full = apply_text(&r.kind, &join_name(&stem, &ext), ctx)?;
                (stem, ext) = split_name(&full, ctx.info.is_dir);
            }
        }
    }
    Ok(join_name(&stem, &ext))
}

// ---- Filtres ------------------------------------------------------------------

#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
pub struct Filters {
    /// Masques à inclure (`*.jpg;*.png`), vide = tout.
    pub include: String,
    /// Masques à exclure.
    pub exclude: String,
    pub files: bool,
    pub folders: bool,
    pub regex: bool,
}

impl Default for Filters {
    fn default() -> Self {
        Filters { include: String::new(), exclude: String::new(), files: true, folders: true, regex: false }
    }
}

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

fn mask_match(masks: &str, name: &str, regex: bool) -> bool {
    if regex {
        return RegexBuilder::new(masks).case_insensitive(true).build().map(|r| r.is_match(name)).unwrap_or(false);
    }
    let n: Vec<char> = name.to_lowercase().chars().collect();
    masks.split([';', '|']).map(|m| m.trim().to_lowercase()).filter(|m| !m.is_empty()).any(|m| {
        let m = if m.contains('*') || m.contains('?') { m } else { format!("*{m}*") };
        glob(&m.chars().collect::<Vec<_>>(), &n)
    })
}

impl Filters {
    pub fn pass(&self, name: &str, is_dir: bool) -> bool {
        if (is_dir && !self.folders) || (!is_dir && !self.files) {
            return false;
        }
        if !self.include.trim().is_empty() && !mask_match(&self.include, name, self.regex) {
            return false;
        }
        if !self.exclude.trim().is_empty() && mask_match(&self.exclude, name, self.regex) {
            return false;
        }
        true
    }
}

/// Comparaison « naturelle » (img2 < img10), insensible à la casse.
pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let (a, b): (Vec<char>, Vec<char>) = (a.to_lowercase().chars().collect(), b.to_lowercase().chars().collect());
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if a[i].is_ascii_digit() && b[j].is_ascii_digit() {
            let (si, sj) = (i, j);
            while i < a.len() && a[i].is_ascii_digit() {
                i += 1;
            }
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            let na: String = a[si..i].iter().collect::<String>().trim_start_matches('0').to_string();
            let nb: String = b[sj..j].iter().collect::<String>().trim_start_matches('0').to_string();
            let o = na.len().cmp(&nb.len()).then_with(|| na.cmp(&nb));
            if o != std::cmp::Ordering::Equal {
                return o;
            }
        } else {
            if a[i] != b[j] {
                return a[i].cmp(&b[j]);
            }
            i += 1;
            j += 1;
        }
    }
    (a.len() - i).cmp(&(b.len() - j))
}

// ---- Calcul + validation --------------------------------------------------------

pub struct Preview {
    pub new: String,
    pub err: Option<String>,
}

const RESERVED: &[&str] = &["con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8", "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9"];

fn check_name(n: &str, en: bool) -> Option<String> {
    let t = |fr: &str, e: &str| Some(if en { e.to_string() } else { fr.to_string() });
    if n.is_empty() {
        return t("Nom vide", "Empty name");
    }
    if n.chars().any(|c| matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || c.is_control()) {
        return t("Caractère interdit", "Invalid character");
    }
    if n.ends_with('.') || n.ends_with(' ') {
        return t("Se termine par un point/espace", "Ends with dot/space");
    }
    let stem = n.split('.').next().unwrap_or("").to_lowercase();
    if RESERVED.contains(&stem.as_str()) {
        return t("Nom réservé", "Reserved name");
    }
    if n.chars().count() > 255 {
        return t("Nom trop long", "Name too long");
    }
    None
}

/// Calcule les nouveaux noms des fichiers *actifs* (`active[i]`) et les valide (doublons, existants).
pub fn preview(files: &[FileInfo], active: &[bool], rules: &[RuleItem], en: bool) -> Vec<Preview> {
    let mut out: Vec<Preview> = Vec::with_capacity(files.len());
    let mut idx = 0usize;
    let mut per_folder: std::collections::HashMap<PathBuf, usize> = std::collections::HashMap::new();
    for (f, act) in files.iter().zip(active) {
        if !*act {
            out.push(Preview { new: f.name.clone(), err: None });
            continue;
        }
        let parent = f.path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
        let inf = {
            let e = per_folder.entry(parent).or_insert(0);
            let v = *e;
            *e += 1;
            v
        };
        let ctx = Ctx { info: f, index: idx, index_in_folder: inf };
        idx += 1;
        match apply_rules(rules, &ctx) {
            Ok(n) => {
                let err = if n == f.name { None } else { check_name(&n, en) };
                out.push(Preview { new: n, err });
            }
            Err(e) => out.push(Preview { new: f.name.clone(), err: Some(e) }),
        }
    }
    // Doublons parmi les cibles + collisions avec des fichiers existants non renommés.
    let key = |p: &Path, n: &str| (p.parent().map(|x| x.to_path_buf()).unwrap_or_default(), n.to_lowercase());
    let mut seen: std::collections::HashMap<(PathBuf, String), usize> = std::collections::HashMap::new();
    let sources: std::collections::HashSet<(PathBuf, String)> = files.iter().zip(active).filter(|(_, a)| **a).map(|(f, _)| key(&f.path, &f.name)).collect();
    // 1) les fichiers inchangés occupent déjà leur nom ; 2) les autres ne doivent pas entrer en collision.
    for (i, (f, act)) in files.iter().zip(active).enumerate() {
        if *act && out[i].new == f.name {
            seen.insert(key(&f.path, &f.name), i);
        }
    }
    for (i, (f, act)) in files.iter().zip(active).enumerate() {
        if !*act || out[i].err.is_some() || out[i].new == f.name {
            continue;
        }
        let k = key(&f.path, &out[i].new);
        if seen.insert(k.clone(), i).is_some() {
            out[i].err = Some(if en { "Duplicate target".into() } else { "Doublon de cible".into() });
            continue;
        }
        if !sources.contains(&k) && f.path.with_file_name(&out[i].new).exists() {
            out[i].err = Some(if en { "Already exists".into() } else { "Existe déjà".into() });
        }
    }
    out
}

// ---- Exécution + annulation ------------------------------------------------------------

/// Renomme en deux temps (via noms temporaires) : gère échanges, chaînes et changements de casse.
/// Annule tout en cas d'échec ; renvoie les paires (ancien, nouveau) effectuées.
pub fn execute(pairs: &[(PathBuf, PathBuf)]) -> Result<Vec<(PathBuf, PathBuf)>, String> {
    let mut stage1: Vec<(PathBuf, PathBuf, PathBuf)> = Vec::new(); // (src, tmp, dst)
    let mut stage2: Vec<(PathBuf, PathBuf, PathBuf)> = Vec::new();
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let rollback = |s1: &[(PathBuf, PathBuf, PathBuf)], s2: &[(PathBuf, PathBuf, PathBuf)]| {
        for (_, tmp, dst) in s2.iter().rev() {
            let _ = std::fs::rename(dst, tmp);
        }
        for (src, tmp, _) in s1.iter().rev() {
            let _ = std::fs::rename(tmp, src);
        }
    };
    for (i, (src, dst)) in pairs.iter().enumerate() {
        let tmp = src.with_file_name(format!(".mfren{stamp}_{i}.tmp"));
        if let Err(e) = std::fs::rename(src, &tmp) {
            rollback(&stage1, &stage2);
            return Err(format!("{} : {e}", src.display()));
        }
        stage1.push((src.clone(), tmp, dst.clone()));
    }
    for (src, tmp, dst) in &stage1 {
        if let Err(e) = std::fs::rename(tmp, dst) {
            rollback(&stage1, &stage2);
            return Err(format!("{} : {e}", dst.display()));
        }
        stage2.push((src.clone(), tmp.clone(), dst.clone()));
    }
    Ok(stage1.into_iter().map(|(s, _, d)| (s, d)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(name: &str) -> FileInfo {
        FileInfo { path: PathBuf::from("/x").join(name), name: name.into(), is_dir: false, mtime: Some(UNIX_EPOCH + std::time::Duration::from_secs(1_704_067_200)), ctime: None, size: 0 }
    }

    fn run(rules: Vec<RuleItem>, name: &str) -> String {
        let f = info(name);
        apply_rules(&rules, &Ctx { info: &f, index: 2, index_in_folder: 2 }).unwrap()
    }

    #[test]
    fn rules_basic() {
        let mut r = default_rule(0);
        r.kind = Kind::Insert { text: "IMG_".into(), place: Place::Prefix, from_right: false };
        assert_eq!(run(vec![r], "photo.jpg"), "IMG_photo.jpg");
        let r = RuleItem::new(Kind::Replace { find: "(\\d+)-(\\d+)".into(), with: "$2_$1".into(), case: false, regex: true, which: Which::All });
        assert_eq!(run(vec![r], "12-34.txt"), "34_12.txt");
        let r = RuleItem::new(Kind::Rearrange { delims: " -".into(), pattern: "$2 - $1".into() });
        assert_eq!(run(vec![r], "Artiste - Titre.mp3"), "Titre - Artiste.mp3");
        let r = RuleItem::new(Kind::Delete { from: 1, from_right: false, mode: DelMode::Count, count: 3, until: String::new() });
        assert_eq!(run(vec![r], "abcdef.txt"), "def.txt");
        let r = RuleItem::new(Kind::Delete { from: 1, from_right: true, mode: DelMode::Count, count: 2, until: String::new() });
        assert_eq!(run(vec![r], "abcdef.txt"), "abcd.txt");
    }

    #[test]
    fn rules_more() {
        let r = RuleItem::new(Kind::Case { mode: CaseMode::Title, exceptions: "de, la".into() });
        assert_eq!(run(vec![r], "le tour de la france.TXT"), "Le Tour de la France.TXT");
        let e = RuleItem::new(Kind::Extension { mode: ExtMode::Lower, text: String::new() });
        assert_eq!(run(vec![e], "A.JPG"), "A.jpg");
        let s = RuleItem::new(Kind::Serialize { start: 1, step: 1, pad: 3, place: Place::Suffix, sep: "_".into(), per_folder: false });
        assert_eq!(run(vec![s], "a.txt"), "a_003.txt");
        let d = RuleItem::new(Kind::Date { format: "yyyy-mm-dd".into(), place: Place::Prefix, source: DateSrc::Modified });
        assert_eq!(run(vec![d], "a.txt"), "2024-01-01a.txt");
        let st = RuleItem::new(Kind::Strip { letters: false, digits: false, symbols: false, brackets: true, accents: true, spaces: true, custom: String::new() });
        assert_eq!(run(vec![st], "été  (copie) [1].txt"), "ete.txt");
        let rm = RuleItem::new(Kind::Remove { text: "COPIE".into(), case: false, which: Which::All });
        assert_eq!(run(vec![rm], "a copie.txt"), "a .txt");
        let both = RuleItem { enabled: true, scope: Scope::Both, kind: Kind::Replace { find: ".jpeg".into(), with: ".jpg".into(), case: false, regex: false, which: Which::All } };
        assert_eq!(run(vec![both], "a.jpeg"), "a.jpg");
    }

    #[test]
    fn natural_and_filters() {
        assert_eq!(natural_cmp("img2", "img10"), std::cmp::Ordering::Less);
        let f = Filters { include: "*.jpg;*.png".into(), exclude: "thumb".into(), ..Default::default() };
        assert!(f.pass("a.JPG", false));
        assert!(!f.pass("a.gif", false));
        assert!(!f.pass("thumb1.jpg", false));
    }

    #[test]
    fn exec_swap_and_undo() {
        let d = std::env::temp_dir().join(format!("ffren{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("a.txt"), "A").unwrap();
        std::fs::write(d.join("b.txt"), "B").unwrap();
        let done = execute(&[(d.join("a.txt"), d.join("b.txt")), (d.join("b.txt"), d.join("a.txt"))]).unwrap();
        assert_eq!(std::fs::read_to_string(d.join("a.txt")).unwrap(), "B");
        let back: Vec<_> = done.iter().map(|(s, t)| (t.clone(), s.clone())).collect();
        execute(&back).unwrap();
        assert_eq!(std::fs::read_to_string(d.join("a.txt")).unwrap(), "A");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn preview_validation() {
        let files = vec![info("a.txt"), info("b.txt")];
        let r = RuleItem::new(Kind::Replace { find: "b".into(), with: "a".into(), case: false, regex: false, which: Which::All });
        let p = preview(&files, &[true, true], &[r], false);
        assert!(p[0].err.is_none());
        assert_eq!(p[1].err.as_deref(), Some("Doublon de cible"));
    }
}
