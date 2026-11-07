//! Commandes en langage naturel : l'IA traduit une phrase en filtre + action, le moteur exécute (plan à valider). v0.1.0
use crate::llm::Llm;
use crate::plan::{Item, Origin};
use crate::rules;
use crate::scan::{Facts, Kind};
use serde_json::Value;
use std::path::Path;

#[derive(Clone, Debug, Default)]
pub struct Filter {
    pub exts: Vec<String>,
    pub kinds: Vec<Kind>,
    pub name_contains: String,
    pub name_regex: String,
    pub min_mb: f64,
    pub max_mb: f64,
    pub after: Option<i64>,
    pub before: Option<i64>,
    pub camera: String,
}

#[derive(Clone, Debug)]
pub enum Op {
    Replace { find: String, with: String, regex: bool },
    Case(String),
    Prefix(String),
    Suffix(String),
    DatePrefix(String),
    Number { start: u32, digits: usize, sep: String },
    Remove(String),
}

#[derive(Clone, Debug)]
pub enum Action {
    Search,
    Rename(Vec<Op>),
    Sort(String),
}

#[derive(Clone, Debug)]
pub struct Command {
    pub filter: Filter,
    pub action: Action,
    pub explain: String,
}

fn parse_date(s: &str) -> Option<i64> {
    let p: Vec<u32> = s.split('-').filter_map(|x| x.trim().parse().ok()).collect();
    (p.len() == 3).then(|| crate::epoch(p[0] as i32, p[1], p[2]))
}

fn strs(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default()
}

impl Filter {
    pub fn from_json(v: &Value) -> Filter {
        Filter {
            exts: strs(&v["exts"]).iter().map(|e| e.trim_start_matches('.').to_lowercase()).collect(),
            kinds: strs(&v["kinds"]).iter().filter_map(|k| Kind::from_key(k)).collect(),
            name_contains: v["name_contains"].as_str().unwrap_or("").to_lowercase(),
            name_regex: v["name_regex"].as_str().unwrap_or("").into(),
            min_mb: v["min_mb"].as_f64().unwrap_or(0.0),
            max_mb: v["max_mb"].as_f64().unwrap_or(0.0),
            after: v["after"].as_str().and_then(parse_date),
            before: v["before"].as_str().and_then(parse_date),
            camera: v["camera"].as_str().unwrap_or("").to_lowercase(),
        }
    }
    pub fn matches(&self, f: &Facts) -> bool {
        let mb = f.size as f64 / 1_048_576.0;
        let t = f.taken.unwrap_or(f.mtime);
        (self.exts.is_empty() || self.exts.contains(&f.ext))
            && (self.kinds.is_empty() || self.kinds.contains(&f.kind()))
            && (self.name_contains.is_empty() || f.name().to_lowercase().contains(&self.name_contains))
            && (self.name_regex.is_empty() || regex::Regex::new(&self.name_regex).map(|r| r.is_match(&f.name())).unwrap_or(true))
            && (self.min_mb <= 0.0 || mb >= self.min_mb)
            && (self.max_mb <= 0.0 || mb <= self.max_mb)
            && self.after.map(|a| t >= a).unwrap_or(true)
            && self.before.map(|b| t < b + 86400).unwrap_or(true)
            && (self.camera.is_empty() || f.camera.to_lowercase().contains(&self.camera))
    }
    pub fn describe(&self) -> String {
        let mut v = vec![];
        if !self.kinds.is_empty() { v.push(format!("types : {}", self.kinds.iter().map(|k| k.label()).collect::<Vec<_>>().join("/"))); }
        if !self.exts.is_empty() { v.push(format!("extensions : {}", self.exts.join(","))); }
        if !self.name_contains.is_empty() { v.push(format!("nom contient « {} »", self.name_contains)); }
        if !self.name_regex.is_empty() { v.push(format!("nom ~ {}", self.name_regex)); }
        if self.min_mb > 0.0 { v.push(format!("≥ {} Mo", self.min_mb)); }
        if self.max_mb > 0.0 { v.push(format!("≤ {} Mo", self.max_mb)); }
        if let Some(a) = self.after { let (y, m, d) = crate::civil(a); v.push(format!("depuis {y}-{m:02}-{d:02}")); }
        if let Some(b) = self.before { let (y, m, d) = crate::civil(b); v.push(format!("jusqu'au {y}-{m:02}-{d:02}")); }
        if !self.camera.is_empty() { v.push(format!("appareil ~ {}", self.camera)); }
        if v.is_empty() { "tous les fichiers".into() } else { v.join(" · ") }
    }
}

fn parse_ops(v: &Value) -> Vec<Op> {
    v.as_array().map(|a| a.iter().filter_map(|o| {
        let s = |k: &str| o[k].as_str().unwrap_or("").to_string();
        Some(match o["op"].as_str()? {
            "replace" => Op::Replace { find: s("find"), with: s("with"), regex: o["regex"].as_bool().unwrap_or(false) },
            "case" => Op::Case(s("mode")),
            "prefix" => Op::Prefix(s("text")),
            "suffix" => Op::Suffix(s("text")),
            "date_prefix" => Op::DatePrefix(if o["fmt"].as_str().unwrap_or("").is_empty() { "%Y-%m-%d".into() } else { s("fmt") }),
            "number" => Op::Number { start: o["start"].as_u64().unwrap_or(1) as u32, digits: o["digits"].as_u64().unwrap_or(3) as usize, sep: if o["sep"].is_null() { "_".into() } else { s("sep") } },
            "remove" => Op::Remove(s("chars")),
            _ => return None,
        })
    }).collect()).unwrap_or_default()
}

pub fn from_json(v: &Value) -> Result<Command, String> {
    let filter = Filter::from_json(&v["filter"]);
    let explain = v["explain"].as_str().unwrap_or("").to_string();
    let action = match v["action"].as_str().unwrap_or("search") {
        "rename" => {
            let ops = parse_ops(&v["rename"]);
            if ops.is_empty() { return Err("aucune opération de renommage comprise".into()); }
            Action::Rename(ops)
        }
        "sort" => Action::Sort(v["template"].as_str().filter(|s| !s.is_empty()).ok_or("gabarit de tri manquant")?.to_string()),
        _ => Action::Search,
    };
    Ok(Command { filter, action, explain })
}

const SYSTEM: &str = r#"Tu es le module « commande » d'AutoForge, un outil de gestion de fichiers. Traduis la demande de l'utilisateur en un objet JSON, SANS texte autour :
{"action":"search|rename|sort",
 "filter":{"exts":["jpg"],"kinds":["photo|video|audio|document|archive|rom|autre"],"name_contains":"","name_regex":"","min_mb":0,"max_mb":0,"after":"AAAA-MM-JJ","before":"AAAA-MM-JJ","camera":""},
 "template":"(sort seulement) chemin relatif avec jetons {année} {mois} {mois_nom} {jour} {nom} {ext} {type} {appareil} {artiste} {album} {titre}",
 "rename":[(rename seulement) {"op":"replace","find":"","with":"","regex":false},{"op":"case","mode":"upper|lower|title"},{"op":"prefix","text":""},{"op":"suffix","text":""},{"op":"date_prefix","fmt":"%Y-%m-%d"},{"op":"number","start":1,"digits":3,"sep":"_"},{"op":"remove","chars":""}],
 "explain":"une phrase en français"}
Omets les champs de filtre inutiles. Les renommages portent sur le nom SANS extension. « search » = retrouver, « rename » = renommer, « sort » = ranger/classer dans des dossiers."#;

pub fn parse(llm: &Llm, text: &str) -> Result<Command, String> {
    from_json(&llm.chat_json(SYSTEM, text, &[], 600)?)
}

fn title_case(s: &str) -> String {
    s.split(' ').map(|w| { let mut c = w.chars(); c.next().map(|f| f.to_uppercase().collect::<String>() + &c.as_str().to_lowercase()).unwrap_or_default() }).collect::<Vec<_>>().join(" ")
}

/// Applique les opérations de renommage au nom (sans extension). `idx` = rang du fichier.
pub fn rename_stem(ops: &[Op], f: &Facts, idx: usize) -> String {
    let mut s = f.stem();
    for op in ops {
        match op {
            Op::Replace { find, with, regex } => {
                if find.is_empty() { continue; }
                if *regex {
                    if let Ok(r) = regex::Regex::new(find) { s = r.replace_all(&s, with.as_str()).to_string(); }
                } else {
                    s = s.replace(find.as_str(), with);
                }
            }
            Op::Case(m) => s = match m.as_str() { "upper" => s.to_uppercase(), "lower" => s.to_lowercase(), _ => title_case(&s) },
            Op::Prefix(t) => s = format!("{t}{s}"),
            Op::Suffix(t) => s = format!("{s}{t}"),
            Op::DatePrefix(fmt) => {
                let (y, m, d) = f.date();
                let p = fmt.replace("%Y", &y.to_string()).replace("%m", &format!("{m:02}")).replace("%d", &format!("{d:02}"));
                s = format!("{p} {s}");
            }
            Op::Number { start, digits, sep } => s = format!("{s}{sep}{:0w$}", *start as usize + idx, w = *digits),
            Op::Remove(chars) => s.retain(|c| !chars.contains(c)),
        }
    }
    s.trim().to_string()
}

/// Construit le plan d'une commande rename/sort sur les fichiers filtrés.
pub fn plan(cmd: &Command, files: &[Facts], dest: &Path) -> Vec<Item> {
    let sel: Vec<&Facts> = files.iter().filter(|f| cmd.filter.matches(f)).collect();
    let mut out = vec![];
    for (i, f) in sel.iter().enumerate() {
        let dst = match &cmd.action {
            Action::Search => continue,
            Action::Rename(ops) => {
                let stem = rename_stem(ops, f, i);
                if stem.is_empty() { continue; }
                let n = if f.ext.is_empty() { stem } else { format!("{stem}.{}", f.path.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default()) };
                f.path.with_file_name(crate::sanitize(&n))
            }
            Action::Sort(t) => dest.join(rules::render(t, f, &[])).join(f.name()),
        };
        if dst != f.path {
            out.push(Item { src: f.path.clone(), dst, reason: cmd.explain.clone(), origin: Origin::Cmd, checked: true });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nl_json_to_plan() {
        let v: Value = serde_json::from_str(r#"{"action":"rename","filter":{"kinds":["photo"],"after":"2023-01-01"},"rename":[{"op":"replace","find":"IMG","with":"Photo"},{"op":"number","start":1,"digits":3,"sep":"_"}],"explain":"x"}"#).unwrap();
        let c = from_json(&v).unwrap();
        let f = |n: &str, t: i64| Facts { path: format!("/d/{n}").into(), ext: "jpg".into(), kind: Some(Kind::Photo), taken: Some(t), ..Default::default() };
        let files = vec![f("IMG_1.jpg", crate::epoch(2023, 5, 1)), f("IMG_2.jpg", crate::epoch(2022, 5, 1)), f("IMG_3.jpg", crate::epoch(2024, 1, 1))];
        let p = plan(&c, &files, Path::new("/o"));
        assert_eq!(p.len(), 2);
        assert_eq!(p[1].dst, Path::new("/d/Photo_3_002.jpg"));
    }
}
