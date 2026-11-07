//! ff-auto : moteur d'AutoForge (module IA indépendant de MediaForge/FileForge). v0.1.0
//! Niveau 1 = règles (EXIF, tags, nom, date) ; niveau 2 = IA locale via LM Studio ; identification web / DAT.
pub mod cfg;
pub mod cmd;
pub mod dat;
pub mod job;
pub mod llm;
pub mod plan;
pub mod rules;
pub mod scan;
pub mod web;

/// Date civile (année, mois, jour) depuis un nombre de secondes Unix (UTC).
pub fn civil(secs: i64) -> (i32, u32, u32) {
    let z = secs.div_euclid(86400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    ((y + (m <= 2) as i64) as i32, m, d)
}

/// Secondes Unix (UTC) d'une date civile.
pub fn epoch(y: i32, m: u32, d: u32) -> i64 {
    let y = y as i64 - (m <= 2) as i64;
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = if m > 2 { m - 3 } else { m + 9 } as i64;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    (era * 146_097 + doe - 719_468) * 86400
}

/// Nettoie un composant de chemin (caractères interdits Windows, points/espaces finaux).
pub fn sanitize(s: &str) -> String {
    let t: String = s.chars().map(|c| if matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || c.is_control() { '_' } else { c }).collect();
    let t = t.trim().trim_end_matches(['.', ' ']).to_string();
    if t.is_empty() {
        "Inconnu".into()
    } else {
        t.chars().take(120).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dates() {
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(epoch(2024, 2, 29)), (2024, 2, 29));
        assert_eq!(civil(epoch(1999, 12, 31) + 86399), (1999, 12, 31));
    }
    #[test]
    fn sanit() {
        assert_eq!(sanitize("a/b:c."), "a_b_c");
        assert_eq!(sanitize("  "), "Inconnu");
    }
}
