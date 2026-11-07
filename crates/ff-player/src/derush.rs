//! Outils de derushage : timecodes pro en surimpression et notes horodatées (fichier texte à côté du média). v0.1.0
use std::path::{Path, PathBuf};

pub const MAX_NOTE: usize = 100;

/// Cadence nominale et drop-frame : 29,97 / 59,94 i/s (NTSC) utilisent le drop-frame.
fn nominal(fps: f64) -> (i64, bool) {
    let n = fps.round().max(1.0);
    let ntsc = (n / 1.001 - fps).abs() < 0.02 && (n == 30.0 || n == 60.0);
    (n as i64, ntsc)
}

/// Numéro d'image à l'instant `t` (secondes).
pub fn frame_no(t: f64, fps: f64) -> i64 {
    (t.max(0.0) * fps + 1e-4).floor() as i64
}

/// SMPTE HH:MM:SS:FF ; avec `drop` (si la cadence est 29,97/59,94) : HH:MM:SS;FF.
pub fn smpte(t: f64, fps: f64, drop: bool) -> String {
    let (nom, ntsc) = nominal(fps);
    let mut f = frame_no(t, fps);
    let use_drop = drop && ntsc;
    if use_drop {
        // Algorithme standard : 2 (ou 4) numéros d'image sautés chaque minute, sauf toutes les 10 minutes.
        let d = if nom == 60 { 4 } else { 2 };
        let per_min = nom * 60 - d;
        let per_10 = per_min * 10 + d;
        let (blocks, rem) = (f / per_10, f % per_10);
        f += d * 9 * blocks + if rem > d { d * ((rem - d) / per_min) } else { 0 };
    }
    let ff = f % nom;
    let s = f / nom;
    format!("{:02}:{:02}:{:02}{}{:02}", s / 3600, s / 60 % 60, s % 60, if use_drop { ';' } else { ':' }, ff)
}

/// HH:MM:SS.mmm
pub fn millis(t: f64) -> String {
    let ms = (t.max(0.0) * 1000.0).round() as i64;
    format!("{:02}:{:02}:{:02}.{:03}", ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000)
}

/// Analyse « HH:MM:SS.mmm », « MM:SS », « SS.s » ou « HH:MM:SS:FF » (ce dernier avec `fps`).
pub fn parse_tc(s: &str, fps: f64) -> Option<f64> {
    let s = s.trim().replace(';', ":");
    let parts: Vec<&str> = s.split(':').collect();
    let num = |x: &str| x.trim().parse::<f64>().ok().filter(|v| *v >= 0.0);
    match parts.len() {
        1 => num(parts[0]),
        2 => Some(num(parts[0])? * 60.0 + num(parts[1])?),
        3 => Some(num(parts[0])? * 3600.0 + num(parts[1])? * 60.0 + num(parts[2])?),
        4 => Some(num(parts[0])? * 3600.0 + num(parts[1])? * 60.0 + num(parts[2])? + num(parts[3])? / fps.max(1.0)),
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    pub t: f64,
    pub dur: f64,
    pub text: String,
}

pub fn clean(text: &str) -> String {
    text.replace(['\t', '\r', '\n'], " ").chars().take(MAX_NOTE).collect()
}

/// Fichier de notes d'un média : « film.mp4.notes.txt » à côté du fichier.
pub fn sidecar(media: &Path) -> PathBuf {
    let mut n = media.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    n.push(".notes.txt");
    media.with_file_name(n)
}

pub fn parse(txt: &str) -> Vec<Note> {
    let mut v: Vec<Note> = txt
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let mut p = l.splitn(3, '\t');
            let t = parse_tc(p.next()?, 25.0)?;
            let dur = p.next()?.trim().parse::<f64>().ok().filter(|d| *d > 0.0)?;
            Some(Note { t, dur, text: clean(p.next().unwrap_or("")) })
        })
        .collect();
    v.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap_or(std::cmp::Ordering::Equal));
    v
}

pub fn serialize(notes: &[Note]) -> String {
    let mut s = String::from("# Notes de derushage — timecode<TAB>durée d'affichage (s)<TAB>texte (100 car. max)\n");
    for n in notes {
        s.push_str(&format!("{}\t{}\t{}\n", millis(n.t), n.dur, clean(&n.text)));
    }
    s
}

pub fn load(media: &Path) -> Vec<Note> {
    std::fs::read_to_string(sidecar(media)).map(|s| parse(&s)).unwrap_or_default()
}

/// Enregistre ; sans note, le fichier est retiré pour ne pas laisser de fichier vide.
pub fn save(media: &Path, notes: &[Note]) -> Result<(), String> {
    let p = sidecar(media);
    if notes.is_empty() {
        let _ = std::fs::remove_file(&p);
        return Ok(());
    }
    std::fs::write(&p, serialize(notes)).map_err(|e| format!("{} : {e}", p.display()))
}

/// Notes visibles à l'instant `pos` (début inclus, fin exclue).
pub fn active(notes: &[Note], pos: f64) -> Vec<&Note> {
    notes.iter().filter(|n| pos >= n.t && pos < n.t + n.dur).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn smpte_ndf() {
        assert_eq!(smpte(0.0, 25.0, false), "00:00:00:00");
        assert_eq!(smpte(3661.52, 25.0, false), "01:01:01:13");
        assert_eq!(frame_no(1.0, 24.0), 24);
    }
    #[test]
    fn smpte_drop_frame() {
        let fps = 30000.0 / 1001.0;
        // Image 1800 = 1 min exacte : le drop-frame saute :00 et :01.
        assert_eq!(smpte(1800.0 / fps, fps, true), "00:01:00;02");
        assert_eq!(smpte(1800.0 / fps, fps, false), "00:01:00:00");
        // Dixième minute : pas de saut.
        assert_eq!(smpte(17982.0 / fps, fps, true), "00:10:00;00");
        assert_eq!(smpte(1.0, 25.0, true), "00:00:01:00");
    }
    #[test]
    fn notes_roundtrip() {
        assert_eq!(millis(83.48), "00:01:23.480");
        assert!((parse_tc("00:01:23.480", 25.0).unwrap() - 83.48).abs() < 1e-9);
        assert!((parse_tc("00:00:01:12", 24.0).unwrap() - 1.5).abs() < 1e-9);
        let n = vec![Note { t: 83.48, dur: 5.0, text: "ligne\tavec tab".into() }, Note { t: 2.0, dur: 7.5, text: "x".repeat(150) }];
        let p = parse(&serialize(&n));
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].t, 2.0);
        assert_eq!(p[0].text.chars().count(), 100);
        assert_eq!(p[1].text, "ligne avec tab");
        assert_eq!(active(&p, 3.0).len(), 1);
        assert_eq!(active(&p, 9.5).len(), 0);
        assert_eq!(active(&p, 9.5 - 1e-9).len(), 1);
    }
}
