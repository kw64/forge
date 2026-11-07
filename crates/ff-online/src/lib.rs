//! Vidéos en ligne : analyse d'une URL (vidéo, playlist, chaîne), listes enregistrées, téléchargement.
//! Moteur : yt-dlp (même principe que NewPipe : interroger l'API interne du site, sans clé), mis à jour séparément. v0.9.0
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    pub id: String,
    pub title: String,
    pub url: String,
    pub duration: Option<f64>,
    pub uploader: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Listing {
    pub name: String,
    pub url: String,
    pub entries: Vec<Entry>,
}

fn exe_name() -> &'static str {
    if cfg!(windows) { "yt-dlp.exe" } else { "yt-dlp" }
}

/// Dossier où MediaForge installe yt-dlp (portable ou profil utilisateur).
pub fn bin_dir() -> PathBuf {
    ff_profiles::data_dir().join("bin")
}

pub mod ffbuild;

fn silent(c: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    c
}

/// Cherche yt-dlp : à côté de l'exécutable, dans le dossier de données, puis dans le PATH.
pub fn locate() -> Option<PathBuf> {
    let mut c: Vec<PathBuf> = Vec::new();
    if let Some(d) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        c.push(d.join(exe_name()));
        c.push(d.join("yt-dlp").join(exe_name()));
    }
    c.push(bin_dir().join(exe_name()));
    if let Some(p) = c.into_iter().find(|p| p.is_file()) {
        return Some(p);
    }
    let ok = silent(Command::new(exe_name()).arg("--version")).stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false);
    ok.then(|| PathBuf::from(exe_name()))
}

pub fn version() -> Option<String> {
    let p = locate()?;
    let o = silent(Command::new(p).arg("--version")).output().ok()?;
    Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
}

/// Principaux services pris en charge (liste indicative ; la liste complète vient de `extractors()`).
pub const SERVICES: &[(&str, &str)] = &[
    ("Vidéo", "YouTube (vidéos, playlists, chaînes, shorts, lives), Vimeo, Dailymotion, Facebook, Instagram, TikTok, X/Twitter, Reddit, Twitch (VOD, clips), Rumble, Odysee, PeerTube, Bilibili, Niconico, Streamable, Imgur"),
    ("Audio", "SoundCloud, Bandcamp, Mixcloud, Audiomack, Radio France (podcasts, directs)"),
    ("France / info", "France.tv, Arte, LCP, BFM, Public Sénat, INA, Le Monde, Franceinfo"),
    ("Autres", "TED, BBC iPlayer (selon pays), Arte Concert, Khan Academy, Coursera (extraits), Wikimedia Commons, Archive.org, pages avec vidéo intégrée, liens directs .mp4 / .m3u8 / .mpd"),
];

/// Liste complète des sites connus de la version de yt-dlp installée.
pub fn extractors() -> Result<Vec<String>, String> {
    let p = locate().ok_or("yt-dlp est introuvable")?;
    let o = silent(Command::new(p).arg("--list-extractors")).output().map_err(|e| e.to_string())?;
    let mut v: Vec<String> = String::from_utf8_lossy(&o.stdout)
        .lines()
        .filter(|l| !l.contains("CURRENTLY BROKEN"))
        .map(|l| l.split(':').next().unwrap_or("").trim().to_string())
        .filter(|n| !n.is_empty() && !n.eq_ignore_ascii_case("generic"))
        .collect();
    v.sort_by_key(|n| n.to_lowercase());
    v.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    Ok(v)
}

/// Télécharge la dernière version de yt-dlp dans `bin_dir()` (curl est fourni avec Windows 10+ et Linux).
pub fn install_latest() -> Result<String, String> {
    let dir = bin_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let dest = dir.join(exe_name());
    let url = format!("https://github.com/yt-dlp/yt-dlp/releases/latest/download/{}", exe_name());
    let tmp = dir.join("yt-dlp.download");
    let st = silent(Command::new("curl").args(["-L", "--fail", "-sS", "-o"]).arg(&tmp).arg(&url)).output().map_err(|e| format!("curl : {e}"))?;
    if !st.status.success() {
        let _ = std::fs::remove_file(&tmp);
        return Err(String::from_utf8_lossy(&st.stderr).trim().to_string());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755));
    }
    let _ = std::fs::remove_file(&dest);
    std::fs::rename(&tmp, &dest).map_err(|e| e.to_string())?;
    Ok(version().unwrap_or_default())
}

fn watch_url(id: &str) -> String {
    format!("https://www.youtube.com/watch?v={id}")
}

/// Transforme la sortie JSON de `yt-dlp -J` en liste (une vidéo seule donne une liste d'un élément).
pub fn parse_listing(json: &str, src_url: &str) -> Result<Listing, String> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| format!("réponse illisible : {e}"))?;
    let s = |x: &serde_json::Value, k: &str| x.get(k).and_then(|v| v.as_str()).map(str::to_string);
    let one = |x: &serde_json::Value| -> Option<Entry> {
        let id = s(x, "id")?;
        let url = s(x, "webpage_url").or_else(|| s(x, "url")).filter(|u| u.starts_with("http")).unwrap_or_else(|| watch_url(&id));
        Some(Entry { title: s(x, "title").unwrap_or_else(|| id.clone()), id, url, duration: x.get("duration").and_then(|d| d.as_f64()), uploader: s(x, "uploader").or_else(|| s(x, "channel")) })
    };
    let (name, entries): (String, Vec<Entry>) = match v.get("entries").and_then(|e| e.as_array()) {
        Some(a) => (s(&v, "title").unwrap_or_else(|| "Liste".into()), a.iter().filter(|x| !x.is_null()).filter_map(one).collect()),
        None => (s(&v, "title").unwrap_or_else(|| "Vidéo".into()), one(&v).into_iter().collect()),
    };
    if entries.is_empty() {
        return Err("aucune vidéo trouvée".into());
    }
    Ok(Listing { name, url: src_url.to_string(), entries })
}

/// Analyse une URL (rapide : liste à plat, sans extraire chaque vidéo).
pub fn fetch(url: &str) -> Result<Listing, String> {
    let exe = locate().ok_or("yt-dlp est introuvable (installez-le depuis la fenêtre « En ligne »).")?;
    let o = silent(Command::new(exe).args(["--flat-playlist", "-J", "--no-warnings", "--playlist-end", "1000"]).arg(url)).output().map_err(|e| e.to_string())?;
    if !o.status.success() {
        let err = String::from_utf8_lossy(&o.stderr);
        return Err(err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("échec de l'analyse").trim().to_string());
    }
    parse_listing(&String::from_utf8_lossy(&o.stdout), url)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Quality {
    /// Meilleure vidéo ≤ 1080p en MP4.
    Video1080,
    /// Meilleure qualité disponible.
    VideoBest,
    /// Audio MP3.
    AudioMp3,
}

/// Télécharge une vidéo ; `progress` reçoit 0..100. Retourne le fichier final.
pub fn download(url: &str, out_dir: &Path, ffmpeg_dir: Option<&Path>, q: Quality, progress: &mut dyn FnMut(f32)) -> Result<PathBuf, String> {
    // 1re tentative avec formats choisis ; 2e tentative "au mieux" si YouTube refuse la sélection.
    match download_once(url, out_dir, ffmpeg_dir, q, false, progress) {
        Err(e) if e.contains("format is not available") || e.contains("Requested format") => download_once(url, out_dir, ffmpeg_dir, q, true, progress),
        r => r,
    }
}

fn download_once(url: &str, out_dir: &Path, ffmpeg_dir: Option<&Path>, q: Quality, loose: bool, progress: &mut dyn FnMut(f32)) -> Result<PathBuf, String> {
    let exe = locate().ok_or("yt-dlp est introuvable")?;
    std::fs::create_dir_all(out_dir).map_err(|e| e.to_string())?;
    let mut c = Command::new(exe);
    c.args(["--no-playlist", "--newline", "--no-warnings", "--no-mtime", "--progress", "--progress-template", "PROG %(progress._percent_str)s", "--print", "after_move:FILE %(filepath)s", "-o"]);
    c.env("PYTHONUTF8", "1").env("PYTHONIOENCODING", "utf-8");
    let started = std::time::SystemTime::now();
    c.arg(out_dir.join("%(title).150B [%(id)s].%(ext)s"));
    c.arg("--windows-filenames");
    match (q, loose) {
        (Quality::Video1080, false) => c.args(["-f", "bv*[height<=1080]+ba/b[height<=1080]/b", "--merge-output-format", "mp4"]),
        (Quality::VideoBest, false) => c.args(["-f", "bv*+ba/b", "--merge-output-format", "mkv"]),
        (Quality::Video1080 | Quality::VideoBest, true) => c.args(["--merge-output-format", "mp4"]),
        (Quality::AudioMp3, false) => c.args(["-f", "ba/b", "-x", "--audio-format", "mp3", "--audio-quality", "0"]),
        (Quality::AudioMp3, true) => c.args(["-x", "--audio-format", "mp3", "--audio-quality", "0"]),
    };
    // ffmpeg : dossier explicite seulement s'il existe (sinon yt-dlp cherche dans le PATH).
    if let Some(d) = ffmpeg_dir.filter(|d| !d.as_os_str().is_empty() && d.exists()) {
        c.arg("--ffmpeg-location").arg(d);
    }
    // YouTube récent : un runtime JS (node/deno) est nécessaire pour déchiffrer les formats.
    if let Some(js) = ["node", "deno", "bun"].into_iter().filter(|_| version().map(|v| v.as_str() >= "2025.10").unwrap_or(false)).find(|n| which_bin(n)) {
        c.args(["--js-runtimes", js]);
    }
    c.arg(url).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = silent(&mut c).spawn().map_err(|e| e.to_string())?;
    // stderr lu en parallèle (évite tout blocage et conserve le vrai message d'erreur).
    let errh = child.stderr.take().map(|mut e| {
        std::thread::spawn(move || {
            use std::io::Read;
            let mut s = String::new();
            let _ = e.read_to_string(&mut s);
            s
        })
    });
    let mut file: Option<PathBuf> = None;
    if let Some(out) = child.stdout.take() {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            if let Some(p) = line.strip_prefix("PROG ") {
                if let Ok(f) = p.trim().trim_end_matches('%').trim().parse::<f32>() {
                    progress(f);
                }
            } else if let Some(f) = line.strip_prefix("FILE ") {
                file = Some(PathBuf::from(f.trim()));
            }
        }
    }
    let st = child.wait().map_err(|e| e.to_string())?;
    let err = errh.and_then(|h| h.join().ok()).unwrap_or_default();
    if !st.success() {
        let msg: Vec<&str> = err.lines().filter(|l| l.contains("ERROR")).map(|l| l.trim()).collect();
        let msg = if msg.is_empty() { err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("échec du téléchargement").trim().to_string() } else { msg.join(" | ") };
        return Err(msg);
    }
    if let Some(f) = file.filter(|f| f.exists()) {
        return Ok(f);
    }
    // Repli (chemin affiché mal encodé, extension changée par ffmpeg…) : fichier le plus récent du dossier.
    let newest = std::fs::read_dir(out_dir)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            let t = m.modified().ok()?;
            let n = e.file_name().to_string_lossy().to_string();
            (m.is_file() && t >= started && !n.ends_with(".part") && !n.ends_with(".ytdl")).then_some((t, e.path()))
        })
        .max_by_key(|(t, _)| *t)
        .map(|(_, p)| p);
    newest.ok_or_else(|| format!("fichier téléchargé introuvable {}", err.lines().last().unwrap_or("")))
}

fn which_bin(n: &str) -> bool {
    let mut c = Command::new(n);
    c.arg("--version").stdout(Stdio::null()).stderr(Stdio::null());
    silent(&mut c).status().map(|s| s.success()).unwrap_or(false)
}

// ---- Enregistrement de directs ---------------------------------------------

/// Enregistrement d'un direct via ffmpeg. Le fichier est écrit en MP4 « fragmenté » (lisible même
/// coupé brutalement) puis remuxé proprement (index final, démarrage rapide) à l'arrêt.
pub struct Recorder {
    child: Option<std::process::Child>,
    tmp: PathBuf,
    out: PathBuf,
    ffmpeg: PathBuf,
    pub started: std::time::Instant,
}

fn ffmpeg_exe(dir: Option<&Path>) -> PathBuf {
    let n = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
    dir.map(|d| d.join(n)).filter(|p| p.exists()).unwrap_or_else(|| PathBuf::from(n))
}

/// Le média est-il un direct en cours ? (un direct YouTube peut avoir une durée : la durée du lecteur ne suffit pas)
pub fn is_live(url: &str) -> bool {
    let Some(exe) = locate() else { return false };
    let mut c = Command::new(exe);
    c.args(["--no-playlist", "--no-warnings", "--skip-download", "--print", "%(live_status)s"]).arg(url);
    silent(&mut c).output().map(|o| String::from_utf8_lossy(&o.stdout).lines().any(|l| l.trim() == "is_live")).unwrap_or(false)
}

impl Recorder {
    /// Démarre l'enregistrement du direct `url` dans `out_dir`.
    pub fn start(url: &str, out_dir: &Path, ffmpeg_dir: Option<&Path>) -> Result<Recorder, String> {
        let exe = locate().ok_or("yt-dlp est introuvable")?;
        std::fs::create_dir_all(out_dir).map_err(|e| e.to_string())?;
        let mut c = Command::new(exe);
        c.args(["--no-playlist", "--no-warnings", "-f", "b/bv*+ba", "--print", "%(title).100B", "--print", "urls"]).arg(url);
        let o = silent(&mut c).output().map_err(|e| e.to_string())?;
        if !o.status.success() {
            return Err(String::from_utf8_lossy(&o.stderr).lines().filter(|l| l.contains("ERROR")).collect::<Vec<_>>().join(" | "));
        }
        let text = String::from_utf8_lossy(&o.stdout).to_string();
        let urls: Vec<&str> = text.lines().map(|l| l.trim()).filter(|l| l.starts_with("http")).collect();
        let title = text.lines().map(|l| l.trim()).find(|l| !l.is_empty() && !l.starts_with("http")).unwrap_or("direct");
        if urls.is_empty() {
            return Err("aucun flux trouvé".into());
        }
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let safe: String = title.chars().map(|c| if r#"<>:"/\|?*"#.contains(c) || c.is_control() { '_' } else { c }).collect();
        let out = out_dir.join(format!("{safe} [{stamp}].mp4"));
        let tmp = out_dir.join(format!("{safe} [{stamp}].live.mp4"));
        let ffmpeg = ffmpeg_exe(ffmpeg_dir);
        let mut f = Command::new(&ffmpeg);
        f.args(["-hide_banner", "-loglevel", "error", "-y", "-reconnect", "1", "-reconnect_streamed", "1", "-reconnect_delay_max", "5"]);
        for u in urls.iter().take(2) {
            f.args(["-i", u]);
        }
        if urls.len() >= 2 {
            f.args(["-map", "0:v:0", "-map", "1:a:0"]);
        } else {
            f.args(["-map", "0:v?", "-map", "0:a?"]);
        }
        f.args(["-c", "copy", "-movflags", "frag_keyframe+empty_moov+default_base_moof"]).arg(&tmp);
        let log = std::env::temp_dir().join("mediaforge-rec.log");
        let errf = std::fs::File::create(&log).map(Stdio::from).unwrap_or_else(|_| Stdio::null());
        f.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(errf);
        let mut child = silent(&mut f).spawn().map_err(|e| format!("ffmpeg : {e}"))?;
        // ffmpeg qui s'arrête aussitôt = flux refusé : on le dit tout de suite.
        std::thread::sleep(std::time::Duration::from_millis(1500));
        if let Ok(Some(_)) = child.try_wait() {
            let e = std::fs::read_to_string(&log).unwrap_or_default();
            return Err(format!("ffmpeg : {}", e.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("flux illisible")));
        }
        Ok(Recorder { child: Some(child), tmp, out, ffmpeg, started: std::time::Instant::now() })
    }

    /// Arrête proprement (« q » à ffmpeg), puis finalise le MP4. Renvoie le fichier utilisable.
    pub fn stop(&mut self) -> Result<PathBuf, String> {
        if let Some(mut ch) = self.child.take() {
            use std::io::Write;
            if let Some(mut si) = ch.stdin.take() {
                let _ = si.write_all(b"q\n");
                let _ = si.flush();
            }
            let t0 = std::time::Instant::now();
            loop {
                match ch.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) if t0.elapsed().as_secs() < 20 => std::thread::sleep(std::time::Duration::from_millis(100)),
                    _ => {
                        let _ = ch.kill();
                        let _ = ch.wait();
                        break;
                    }
                }
            }
        }
        if !self.tmp.metadata().map(|m| m.len() > 0).unwrap_or(false) {
            let e = std::fs::read_to_string(std::env::temp_dir().join("mediaforge-rec.log")).unwrap_or_default();
            return Err(format!("aucun fichier enregistré {}", e.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("")));
        }
        // Remux : index final + faststart ; en cas d'échec on garde le MP4 fragmenté (lisible).
        let mut r = Command::new(&self.ffmpeg);
        r.args(["-hide_banner", "-loglevel", "error", "-y", "-i"]).arg(&self.tmp).args(["-c", "copy", "-movflags", "+faststart"]).arg(&self.out).stdin(Stdio::null());
        let ok = silent(&mut r).status().map(|s| s.success()).unwrap_or(false) && self.out.metadata().map(|m| m.len() > 0).unwrap_or(false);
        if ok {
            let _ = std::fs::remove_file(&self.tmp);
            Ok(self.out.clone())
        } else {
            let _ = std::fs::remove_file(&self.out);
            let keep = self.out.with_file_name(self.tmp.file_name().unwrap_or_default().to_string_lossy().replace(".live.mp4", " (fragmenté).mp4"));
            std::fs::rename(&self.tmp, &keep).map_err(|e| e.to_string())?;
            Ok(keep)
        }
    }
}

impl Drop for Recorder {
    /// Fermeture du lecteur en pleine capture : on finalise quand même.
    fn drop(&mut self) {
        if self.child.is_some() {
            let _ = self.stop();
        }
    }
}

// ---- Listes enregistrées -------------------------------------------------

fn lists_path() -> PathBuf {
    ff_profiles::data_dir().join("online.json")
}

pub fn load_lists() -> Vec<Listing> {
    std::fs::read_to_string(lists_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub fn save_lists(l: &[Listing]) -> Result<(), String> {
    let p = lists_path();
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    std::fs::write(p, serde_json::to_string_pretty(l).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

/// Liste au format M3U (lisible par tout lecteur compatible yt-dlp/mpv).
pub fn to_m3u(l: &Listing) -> String {
    let mut s = String::from("#EXTM3U\n");
    for e in &l.entries {
        s.push_str(&format!("#EXTINF:{},{}\n{}\n", e.duration.map(|d| d as i64).unwrap_or(-1), e.title.replace('\n', " "), e.url));
    }
    s
}

pub fn is_url(s: &str) -> bool {
    s.starts_with("http://") || s.starts_with("https://")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playlist_json() {
        let j = r#"{"title":"Mes vidéos","entries":[{"id":"abc","title":"Un","url":"https://www.youtube.com/watch?v=abc","duration":61.0,"uploader":"Moi"},{"id":"def","title":"Deux","url":"def"},null]}"#;
        let l = parse_listing(j, "https://www.youtube.com/playlist?list=X").unwrap();
        assert_eq!(l.name, "Mes vidéos");
        assert_eq!(l.entries.len(), 2);
        assert_eq!(l.entries[1].url, "https://www.youtube.com/watch?v=def");
        assert_eq!(l.entries[0].duration, Some(61.0));
    }

    #[test]
    fn single_video_json() {
        let l = parse_listing(r#"{"id":"zzz","title":"Seule","webpage_url":"https://youtu.be/zzz"}"#, "u").unwrap();
        assert_eq!(l.entries.len(), 1);
        assert_eq!(l.entries[0].url, "https://youtu.be/zzz");
        assert!(parse_listing(r#"{"entries":[]}"#, "u").is_err());
    }

    #[test]
    fn m3u() {
        let l = Listing { name: "n".into(), url: "u".into(), entries: vec![Entry { id: "a".into(), title: "T".into(), url: "https://x/a".into(), duration: Some(5.0), uploader: None }] };
        assert!(to_m3u(&l).contains("#EXTINF:5,T\nhttps://x/a"));
    }
}

#[cfg(test)]
mod rec_tests {
    use super::*;

    /// Simule un direct (source infinie) : l'arrêt doit produire un MP4 complet et lisible.
    #[test]
    fn stop_finalizes() {
        if Command::new("ffmpeg").arg("-version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_err() {
            return;
        }
        let d = std::env::temp_dir().join(format!("ffrec{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let tmp = d.join("t.live.mp4");
        let child = Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y", "-re", "-f", "lavfi", "-i", "testsrc=size=160x120:rate=10", "-c:v", "libx264", "-g", "5", "-movflags", "frag_keyframe+empty_moov+default_base_moof"])
            .arg(&tmp)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut r = Recorder { child: Some(child), tmp, out: d.join("t.mp4"), ffmpeg: PathBuf::from("ffmpeg"), started: std::time::Instant::now() };
        std::thread::sleep(std::time::Duration::from_secs(3));
        let f = r.stop().unwrap();
        let o = Command::new("ffprobe").args(["-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0"]).arg(&f).output().unwrap();
        let dur: f64 = String::from_utf8_lossy(&o.stdout).trim().parse().unwrap_or(0.0);
        assert!(dur > 1.5, "durée {dur}");
        assert!(!d.join("t.live.mp4").exists());
        let _ = std::fs::remove_dir_all(&d);
    }
}
