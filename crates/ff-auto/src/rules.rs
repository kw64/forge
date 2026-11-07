//! Niveau 1 : règles de tri, gabarits de chemin, doublons. v0.1.0
use crate::scan::{Facts, Kind};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Rule {
    pub name: String,
    pub enabled: bool,
    pub kinds: Vec<Kind>,
    /// Extensions (sans point, minuscules), vide = toutes.
    pub exts: String,
    pub name_regex: String,
    pub min_mb: f64,
    pub max_mb: f64,
    /// 0 = indifférent, 1 = doit avoir une date EXIF/nom, 2 = ne doit pas en avoir.
    pub need_date: u8,
    pub template: String,
}

impl Default for Rule {
    fn default() -> Self {
        Rule { name: "Nouvelle règle".into(), enabled: true, kinds: vec![], exts: String::new(), name_regex: String::new(), min_mb: 0.0, max_mb: 0.0, need_date: 0, template: "Divers/{nom}".into() }
    }
}

pub fn default_rules() -> Vec<Rule> {
    let r = |name: &str, kinds: Vec<Kind>, need_date: u8, template: &str| Rule { name: name.into(), kinds, need_date, template: template.into(), ..Default::default() };
    vec![
        Rule { name: "Captures d'écran".into(), kinds: vec![Kind::Photo], name_regex: "(?i)(screenshot|capture|screen ?shot)".into(), template: "Photos/Captures d'écran/{année}".into(), ..Default::default() },
        r("Photos datées", vec![Kind::Photo], 1, "Photos/{année}/{année}-{mois}"),
        r("Photos sans date", vec![Kind::Photo], 2, "Photos/Sans date"),
        r("Vidéos", vec![Kind::Video], 0, "Vidéos/{année}"),
        r("Musique", vec![Kind::Audio], 0, "Musique/{artiste}/{album}"),
        r("Jeux", vec![Kind::Rom], 0, "Jeux/{ext}"),
        r("Documents", vec![Kind::Doc], 0, "Documents/{année}"),
        r("Archives", vec![Kind::Archive], 0, "Archives"),
        r("Autres", vec![Kind::Other], 0, "Divers"),
    ]
}

impl Rule {
    pub fn matches(&self, f: &Facts) -> bool {
        if !self.enabled {
            return false;
        }
        if !self.kinds.is_empty() && !self.kinds.contains(&f.kind()) {
            return false;
        }
        let exts: Vec<String> = self.exts.split([',', ';', ' ']).map(|s| s.trim().trim_start_matches('.').to_lowercase()).filter(|s| !s.is_empty()).collect();
        if !exts.is_empty() && !exts.contains(&f.ext) {
            return false;
        }
        if !self.name_regex.is_empty() && !regex::Regex::new(&self.name_regex).map(|r| r.is_match(&f.name())).unwrap_or(false) {
            return false;
        }
        let mb = f.size as f64 / 1_048_576.0;
        if self.min_mb > 0.0 && mb < self.min_mb || self.max_mb > 0.0 && mb > self.max_mb {
            return false;
        }
        match self.need_date {
            1 if f.taken.is_none() => false,
            2 if f.taken.is_some() => false,
            _ => true,
        }
    }
}

pub fn first_match<'a>(rules: &'a [Rule], f: &Facts) -> Option<&'a Rule> {
    rules.iter().find(|r| r.matches(f))
}

pub const MONTHS: [&str; 12] = ["janvier", "février", "mars", "avril", "mai", "juin", "juillet", "août", "septembre", "octobre", "novembre", "décembre"];

/// Jetons disponibles (affichés dans l'aide de l'interface).
pub const TOKENS: &str = "{année} {mois} {mois_nom} {jour} {nom} {ext} {type} {appareil} {artiste} {album} {titre} {piste} {categorie}";

/// Évalue un gabarit de chemin. `extra` apporte des jetons supplémentaires (catégorie, titre web…).
pub fn render(template: &str, f: &Facts, extra: &[(&str, String)]) -> String {
    let (y, m, d) = f.date();
    let get = |k: &str| -> Option<String> {
        if let Some((_, v)) = extra.iter().find(|(n, _)| *n == k) {
            return Some(v.clone());
        }
        Some(match k {
            "année" | "annee" | "year" => y.to_string(),
            "mois" | "month" => format!("{m:02}"),
            "mois_nom" => MONTHS[(m - 1) as usize].to_string(),
            "jour" | "day" => format!("{d:02}"),
            "nom" | "name" => f.stem(),
            "ext" => f.ext.clone(),
            "type" => f.kind().label().to_string(),
            "appareil" | "camera" => f.camera.clone(),
            "artiste" | "artist" => f.artist.clone(),
            "album" => f.album.clone(),
            "titre" | "title" => if f.title.is_empty() { f.stem() } else { f.title.clone() },
            "piste" | "track" => if f.track > 0 { format!("{:02}", f.track) } else { String::new() },
            "categorie" | "catégorie" => String::new(),
            _ => return None,
        })
    };
    let mut out = String::new();
    let mut rest = template;
    while let Some(i) = rest.find('{') {
        out.push_str(&rest[..i]);
        if let Some(j) = rest[i..].find('}') {
            let key = &rest[i + 1..i + j];
            out.push_str(&get(key).unwrap_or_default());
            rest = &rest[i + j + 1..];
        } else {
            rest = &rest[i..];
            break;
        }
    }
    out.push_str(rest);
    // Composants vides -> « Inconnu » ; interdits retirés.
    out.split(['/', '\\']).map(|s| s.trim()).filter(|_| true).map(|s| if s.is_empty() { "Inconnu".to_string() } else { crate::sanitize(s) }).collect::<Vec<_>>().join("/")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn photo(name: &str, taken: Option<i64>) -> Facts {
        Facts { path: name.into(), ext: "jpg".into(), kind: Some(Kind::Photo), taken, mtime: crate::epoch(2020, 1, 2), size: 3 << 20, ..Default::default() }
    }
    #[test]
    fn template_and_rules() {
        let f = photo("IMG_1.jpg", Some(crate::epoch(2023, 7, 14)));
        assert_eq!(render("Photos/{année}/{année}-{mois}/{nom}", &f, &[]), "Photos/2023/2023-07/IMG_1");
        assert_eq!(render("M/{artiste}/{album}", &f, &[]), "M/Inconnu/Inconnu");
        let rules = default_rules();
        assert_eq!(first_match(&rules, &f).unwrap().name, "Photos datées");
        assert_eq!(first_match(&rules, &photo("Screenshot_5.jpg", None)).unwrap().name, "Captures d'écran");
        assert_eq!(first_match(&rules, &photo("x.jpg", None)).unwrap().name, "Photos sans date");
    }
}
