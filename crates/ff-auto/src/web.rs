//! Identification par Internet (TMDB, iTunes, MusicBrainz, Wikipédia) et par DAT (ROMs). v0.1.0
use crate::cfg::Config;
use crate::dat::DatIndex;
use crate::llm::Llm;
use crate::scan::{Facts, Kind};
use serde_json::Value;
use std::time::Duration;

#[derive(Clone, Debug, Default)]
pub struct Cand {
    pub title: String,
    pub year: String,
    pub artist: String,
    pub album: String,
    pub source: String,
    pub score: f32,
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new().timeout(Duration::from_secs(15)).user_agent("AutoForge/0.1 (SIVOM file organiser)").build()
}

fn get(url: &str, q: &[(&str, &str)]) -> Result<Value, String> {
    let mut r = agent().get(url);
    for (k, v) in q {
        r = r.query(k, v);
    }
    r.call().map_err(|e| format!("{url} : {e}"))?.into_json().map_err(|e| e.to_string())
}

const JUNK: &[&str] = &["1080p", "720p", "2160p", "480p", "4k", "uhd", "bluray", "blu-ray", "brrip", "bdrip", "webrip", "web-dl", "webdl", "dvdrip", "hdtv", "hdrip", "x264", "x265", "h264", "h265", "hevc", "xvid", "divx", "multi", "vff", "vfq", "vf", "vostfr", "truefrench", "french", "subfrench", "aac", "ac3", "dts", "remux", "proper", "extended", "repack", "complete"];

/// « Mon.Film.2019.1080p.BluRay.x264-GRP » -> (« Mon Film », Some(2019)).
pub fn clean_name(stem: &str) -> (String, Option<u32>) {
    let year = regex::Regex::new(r"[\(\[\. _-]((?:19|20)\d{2})[\)\]\. _-]?").ok().and_then(|r| r.captures_iter(stem).last().and_then(|c| c[1].parse::<u32>().ok()));
    let mut s = regex::Regex::new(r"\[[^\]]*\]|\([^\)]*\)").map(|r| r.replace_all(stem, " ").to_string()).unwrap_or_else(|_| stem.to_string());
    s = s.replace(['.', '_'], " ");
    let mut out = vec![];
    for w in s.split_whitespace() {
        let lw = w.to_lowercase();
        if JUNK.contains(&lw.as_str()) || (year.is_some() && lw == year.unwrap().to_string()) {
            if JUNK.contains(&lw.as_str()) {
                break;
            }
            continue;
        }
        out.push(w);
    }
    (out.join(" ").trim_matches(['-', ' ']).to_string(), year)
}

fn norm(s: &str) -> Vec<String> {
    let fold = |c: char| match c { 'é' | 'è' | 'ê' | 'ë' => 'e', 'à' | 'â' | 'ä' => 'a', 'î' | 'ï' => 'i', 'ô' | 'ö' => 'o', 'ù' | 'û' | 'ü' => 'u', 'ç' => 'c', c => c };
    s.to_lowercase().chars().map(fold).map(|c| if c.is_alphanumeric() { c } else { ' ' }).collect::<String>().split_whitespace().map(String::from).collect()
}

/// Similarité de jetons (Jaccard) entre deux titres, 0..1.
pub fn similarity(a: &str, b: &str) -> f32 {
    let (x, y) = (norm(a), norm(b));
    if x.is_empty() || y.is_empty() {
        return 0.0;
    }
    let inter = x.iter().filter(|t| y.contains(t)).count() as f32;
    inter / (x.len() + y.len()) as f32 * 2.0 / 1.0 * 0.5 + inter / x.len().max(y.len()) as f32 * 0.5
}

fn year4(s: &str) -> String {
    s.chars().take(4).collect()
}

pub fn tmdb_movie(q: &str, key: &str, lang: &str) -> Result<Vec<Cand>, String> {
    let l = if lang == "en" { "en-US" } else { "fr-FR" };
    let v = get("https://api.themoviedb.org/3/search/movie", &[("api_key", key), ("query", q), ("language", l)])?;
    Ok(v["results"].as_array().map(|a| a.iter().take(6).map(|m| Cand { title: m["title"].as_str().unwrap_or("").into(), year: year4(m["release_date"].as_str().unwrap_or("")), source: "TMDB".into(), ..Default::default() }).collect()).unwrap_or_default())
}

pub fn itunes_movie(q: &str, lang: &str) -> Result<Vec<Cand>, String> {
    let c = if lang == "en" { "us" } else { "fr" };
    let v = get("https://itunes.apple.com/search", &[("term", q), ("media", "movie"), ("entity", "movie"), ("limit", "6"), ("country", c)])?;
    Ok(v["results"].as_array().map(|a| a.iter().map(|m| Cand { title: m["trackName"].as_str().unwrap_or("").into(), year: year4(m["releaseDate"].as_str().unwrap_or("")), artist: m["artistName"].as_str().unwrap_or("").into(), source: "iTunes".into(), ..Default::default() }).collect()).unwrap_or_default())
}

pub fn wikipedia(q: &str, lang: &str) -> Result<Vec<Cand>, String> {
    let l = if lang == "en" { "en" } else { "fr" };
    let v = get(&format!("https://{l}.wikipedia.org/w/api.php"), &[("action", "query"), ("list", "search"), ("srsearch", q), ("srlimit", "4"), ("format", "json")])?;
    Ok(v["query"]["search"].as_array().map(|a| a.iter().map(|m| Cand { title: m["title"].as_str().unwrap_or("").into(), source: "Wikipédia".into(), ..Default::default() }).collect()).unwrap_or_default())
}

pub fn musicbrainz(artist: &str, title: &str) -> Result<Vec<Cand>, String> {
    let mut q = format!("recording:\"{}\"", title.replace('"', ""));
    if !artist.is_empty() {
        q.push_str(&format!(" AND artist:\"{}\"", artist.replace('"', "")));
    }
    let v = get("https://musicbrainz.org/ws/2/recording/", &[("query", &q), ("fmt", "json"), ("limit", "6")])?;
    std::thread::sleep(Duration::from_millis(1100)); // limite MusicBrainz : 1 requête/s
    Ok(v["recordings"].as_array().map(|a| a.iter().map(|m| Cand {
        title: m["title"].as_str().unwrap_or("").into(),
        artist: m["artist-credit"][0]["name"].as_str().unwrap_or("").into(),
        album: m["releases"][0]["title"].as_str().unwrap_or("").into(),
        year: year4(m["first-release-date"].as_str().unwrap_or("")),
        source: "MusicBrainz".into(),
        ..Default::default()
    }).collect()).unwrap_or_default())
}

#[derive(Clone, Debug)]
pub struct Ident {
    pub new_stem: String,
    pub detail: String,
    pub score: f32,
    pub cand: Cand,
}

fn fill(t: &str, c: &Cand) -> String {
    crate::sanitize(&t.replace("{titre}", &c.title).replace("{année}", &c.year).replace("{artiste}", &c.artist).replace("{album}", &c.album).replace(" ()", "").replace("()", ""))
}

/// Identifie un fichier (film/série, musique, ROM) et propose un nom propre.
pub fn identify(f: &Facts, cfg: &Config, llm: Option<&Llm>, dat: &DatIndex) -> Result<Option<Ident>, String> {
    match f.kind() {
        Kind::Rom => {
            if dat.is_empty() {
                return Err("aucun fichier DAT chargé (Réglages › DAT)".into());
            }
            if let Some((crc, size)) = crate::dat::rom_crc(&f.path) {
                if let Some(e) = dat.lookup(crc, size) {
                    let c = Cand { title: e.game.clone(), source: "DAT (CRC32)".into(), score: 1.0, ..Default::default() };
                    return Ok(Some(Ident { new_stem: crate::sanitize(&e.game), detail: format!("CRC {crc:08X} reconnu"), score: 1.0, cand: c }));
                }
            }
            // Repli : meilleur nom approchant dans les DAT.
            let (q, _) = clean_name(&f.stem());
            let best = dat.games.iter().map(|g| (similarity(&q, g), g)).max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
            Ok(best.filter(|(s, _)| *s >= 0.6).map(|(s, g)| Ident { new_stem: crate::sanitize(g), detail: format!("nom approchant ({:.0} %)", s * 100.0), score: s * 0.9, cand: Cand { title: g.clone(), source: "DAT (nom)".into(), score: s, ..Default::default() } }))
        }
        Kind::Video => {
            let (mut q, mut year) = clean_name(&f.stem());
            if let (Some(l), true) = (llm, cfg.llm_title) {
                if let Ok(v) = l.chat_json("Extrais le titre d'un film ou d'une série depuis un nom de fichier. JSON uniquement : {\"titre\":\"\",\"annee\":null}", &f.stem(), &[], 100) {
                    if let Some(t) = v["titre"].as_str().filter(|t| !t.is_empty()) { q = t.to_string(); }
                    year = v["annee"].as_u64().map(|y| y as u32).or(year);
                }
            }
            if q.is_empty() { return Ok(None); }
            let mut cands = if cfg.tmdb_key.is_empty() { itunes_movie(&q, &cfg.web_lang).unwrap_or_default() } else { tmdb_movie(&q, &cfg.tmdb_key, &cfg.web_lang)? };
            if cands.is_empty() { cands = wikipedia(&format!("{q} film"), &cfg.web_lang).unwrap_or_default(); }
            for c in &mut cands {
                c.score = similarity(&q, &c.title) + if year.map(|y| c.year == y.to_string()).unwrap_or(false) { 0.1 } else { 0.0 };
            }
            let Some(b) = cands.into_iter().max_by(|a, b| a.score.partial_cmp(&b.score).unwrap_or(std::cmp::Ordering::Equal)).filter(|c| c.score >= 0.5) else { return Ok(None) };
            let mut b = b;
            if b.year.is_empty() { if let Some(y) = year { b.year = y.to_string(); } }
            Ok(Some(Ident { new_stem: fill(&cfg.film_template, &b), detail: format!("{} · requête « {q} »", b.source), score: b.score.min(1.0), cand: b }))
        }
        Kind::Audio => {
            let (mut artist, mut title) = (f.artist.clone(), f.title.clone());
            if title.is_empty() {
                let (q, _) = clean_name(&f.stem());
                match q.split_once(" - ") {
                    Some((a, t)) if artist.is_empty() => { artist = a.trim().into(); title = t.trim().into(); }
                    _ => title = q,
                }
            }
            if title.is_empty() { return Ok(None); }
            let mut cands = musicbrainz(&artist, &title)?;
            for c in &mut cands {
                c.score = similarity(&title, &c.title) * 0.8 + if artist.is_empty() { 0.1 } else { similarity(&artist, &c.artist) * 0.2 };
            }
            let Some(b) = cands.into_iter().max_by(|a, b| a.score.partial_cmp(&b.score).unwrap_or(std::cmp::Ordering::Equal)).filter(|c| c.score >= 0.6) else { return Ok(None) };
            Ok(Some(Ident { new_stem: fill(&cfg.music_template, &b), detail: format!("MusicBrainz · album « {} »", b.album), score: b.score.min(1.0), cand: b }))
        }
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cleaning() {
        let (n, y) = clean_name("Le.Fabuleux.Destin.2001.1080p.BluRay.x264-GRP");
        assert_eq!((n.as_str(), y), ("Le Fabuleux Destin", Some(2001)));
        let (n, y) = clean_name("Blade Runner (1982) [MULTI]");
        assert_eq!((n.as_str(), y), ("Blade Runner", Some(1982)));
    }
    #[test]
    fn sim() {
        assert!(similarity("Le Fabuleux Destin d'Amélie Poulain", "Le fabuleux destin d'Amelie Poulain") > 0.95);
        assert!(similarity("Alien", "Titanic") < 0.1);
    }
}
