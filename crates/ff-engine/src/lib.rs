//! ff-engine : localisation de FFmpeg, construction des arguments, exécution, file d'attente. v0.3.0
use anyhow::{anyhow, bail, Context, Result};
use ff_core::{args::codec_args, HwAccel, Job, JobKind, Progress};
use std::ffi::OsString;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub mod queue;
pub use queue::Queue;

/// Erreur renvoyée par `run` quand la tâche est annulée.
#[derive(Debug)]
pub struct Cancelled;
impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "annulé")
    }
}
impl std::error::Error for Cancelled {}

/// Chemins des binaires FFmpeg.
#[derive(Debug, Clone)]
pub struct Binaries {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
}

fn exe(name: &str) -> String {
    if cfg!(windows) { format!("{name}.exe") } else { name.to_string() }
}

/// Évite l'ouverture d'une fenêtre console noire sous Windows.
fn quiet(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd
}

fn runs(bin: &Path) -> bool {
    quiet(Command::new(bin).arg("-version"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Ordre de recherche : dossier explicite (réglage / option), variable MEDIAFORGE_FFMPEG_DIR,
/// dossier `ffmpeg/` à côté de l'exécutable (AppImage, zip portable), puis PATH.
pub fn locate(explicit_dir: Option<&Path>) -> Result<Binaries> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(d) = explicit_dir {
        dirs.push(d.to_path_buf());
    }
    if let Some(d) = std::env::var_os("MEDIAFORGE_FFMPEG_DIR") {
        dirs.push(PathBuf::from(d));
    }
    // Dans une AppImage, les binaires embarqués sont sous $APPDIR/usr/bin.
    if let Some(d) = std::env::var_os("APPDIR") {
        dirs.push(PathBuf::from(d).join("usr/bin"));
    }
    if let Ok(me) = std::env::current_exe() {
        if let Some(parent) = me.parent() {
            dirs.push(parent.join("ffmpeg"));
            dirs.push(parent.to_path_buf());
        }
    }
    for d in &dirs {
        let (ff, fp) = (d.join(exe("ffmpeg")), d.join(exe("ffprobe")));
        if runs(&ff) && runs(&fp) {
            return Ok(Binaries { ffmpeg: ff, ffprobe: fp });
        }
    }
    let (ff, fp) = (PathBuf::from(exe("ffmpeg")), PathBuf::from(exe("ffprobe")));
    if runs(&ff) && runs(&fp) {
        return Ok(Binaries { ffmpeg: ff, ffprobe: fp });
    }
    bail!("FFmpeg/FFprobe introuvables (PATH, dossier ffmpeg/ ou MEDIAFORGE_FFMPEG_DIR)")
}

/// Version de FFmpeg (première ligne de `-version`).
pub fn version(b: &Binaries) -> Result<String> {
    let out = quiet(Command::new(&b.ffmpeg).arg("-version")).output()?;
    Ok(String::from_utf8_lossy(&out.stdout).lines().next().unwrap_or("").to_string())
}

/// Détecte les encodeurs matériels compilés dans FFmpeg (leur présence ne garantit pas le matériel).
pub fn available_hw(b: &Binaries) -> Vec<HwAccel> {
    let out = quiet(Command::new(&b.ffmpeg).args(["-hide_banner", "-encoders"])).output();
    let txt = out.map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    HwAccel::ALL
        .into_iter()
        .filter(|h| *h == HwAccel::None || txt.contains(&format!("h264_{}", h.suffix())))
        .collect()
}

/// Noms des encodeurs disponibles dans cette version de FFmpeg (pour griser les codecs absents).
pub fn available_encoders(b: &Binaries) -> std::collections::HashSet<String> {
    let out = quiet(Command::new(&b.ffmpeg).args(["-hide_banner", "-encoders"])).output();
    let txt = out.map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    txt.lines().filter_map(|l| {
        let mut it = l.split_whitespace();
        let flags = it.next()?;
        (flags.len() == 6 && flags.starts_with('V') || flags.len() == 6 && flags.starts_with('A')).then(|| it.next().map(str::to_string)).flatten()
    }).collect()
}

/// Commande préparée : arguments, fichier temporaire éventuel, durée pour la progression.
pub struct Plan {
    pub args: Vec<OsString>,
    pub tmp: Option<PathBuf>,
    pub duration: Option<f64>,
    pub hw_applied: bool,
}

const HEAD: [&str; 6] = ["-hide_banner", "-nostdin", "-y", "-progress", "pipe:1", "-nostats"];

fn path_arg(p: &Path) -> OsString {
    p.as_os_str().to_os_string()
}

fn dur(bins: &Binaries, p: &Path) -> Result<Option<f64>> {
    Ok(ff_probe::probe(&bins.ffprobe, p)?.duration_secs)
}

/// Construit la commande FFmpeg d'une tâche. `software` force le repli sans accélération matérielle.
pub fn plan(bins: &Binaries, job: &Job, software: bool) -> Result<Plan> {
    let hw = if software { HwAccel::None } else { job.hw };
    let mut args: Vec<OsString> = HEAD.iter().map(OsString::from).collect();
    let mut tmp = None;
    let (duration, hw_applied);

    match &job.kind {
        JobKind::Convert => {
            let info = ff_probe::probe(&bins.ffprobe, &job.input)?;
            if !info.has_video && !info.has_audio {
                bail!("aucun flux audio ou vidéo dans {}", job.input.display());
            }
            if job.profile.is_image() && !info.has_video {
                bail!("{} ne contient pas d'image", job.input.display());
            }
            if let Some(s) = job.edits.trim_start.filter(|s| *s > 0.0) {
                args.extend(["-ss".into(), format!("{s:.3}").into()]);
            }
            args.push("-i".into());
            args.push(path_arg(&job.input));
            let (codec, applied) = codec_args(&job.profile, hw, &job.edits).map_err(|e| anyhow!(e))?;
            // Cadence réelle du fichier pour le timecode incrusté (« {FPS} »).
            let fps = info.fps.clone().unwrap_or_else(|| "25".into());
            args.extend(codec.iter().map(|c| OsString::from(c.replace("{FPS}", &fps))));
            let span = job.edits.span(info.duration_secs);
            if job.edits.trim_end.is_some() {
                if let Some(d) = span {
                    if d <= 0.0 {
                        bail!("la fin de la découpe doit être après le début");
                    }
                    args.extend(["-t".into(), format!("{d:.3}").into()]);
                }
            }
            duration = span;
            hw_applied = applied;
        }
        JobKind::Concat { inputs, copy } => {
            if inputs.len() < 2 {
                bail!("la fusion demande au moins deux fichiers");
            }
            let mut total = Some(0.0);
            for i in inputs {
                total = match (total, dur(bins, i)?) {
                    (Some(t), Some(d)) => Some(t + d),
                    _ => None,
                };
            }
            let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
            let list = std::env::temp_dir().join(format!("mediaforge_concat_{}_{nanos}.txt", std::process::id()));
            let body: String = inputs
                .iter()
                .map(|p| format!("file '{}'\n", p.to_string_lossy().replace('\\', "/").replace('\'', "'\\''")))
                .collect();
            std::fs::write(&list, body).context("écriture de la liste de fusion")?;
            args.extend(["-f".into(), "concat".into(), "-safe".into(), "0".into(), "-i".into(), path_arg(&list)]);
            tmp = Some(list);
            if *copy {
                args.extend(["-c".into(), "copy".into()]);
                hw_applied = false;
            } else {
                let (codec, applied) = codec_args(&job.profile, hw, &ff_core::Edits::default()).map_err(|e| anyhow!(e))?;
                args.extend(codec.iter().map(OsString::from));
                hw_applied = applied;
            }
            duration = total;
        }
        JobKind::Mux { audio } => {
            let ext = job.output.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
            let acodec = match ext.as_str() {
                "mp4" | "m4v" | "mov" => "aac",
                "webm" => "libopus",
                _ => "copy",
            };
            args.push("-i".into());
            args.push(path_arg(&job.input));
            args.push("-i".into());
            args.push(path_arg(audio));
            args.extend(["-map", "0:v:0", "-map", "1:a:0", "-c:v", "copy", "-c:a", acodec, "-shortest"].map(OsString::from));
            duration = dur(bins, &job.input)?;
            hw_applied = false;
        }
        JobKind::Mix { inputs } => {
            if inputs.len() < 2 {
                bail!("le mixage demande au moins deux fichiers");
            }
            let mut longest: Option<f64> = Some(0.0);
            for i in inputs {
                args.push("-i".into());
                args.push(path_arg(i));
                longest = match (longest, dur(bins, i)?) {
                    (Some(a), Some(b)) => Some(a.max(b)),
                    _ => None,
                };
            }
            args.extend(["-filter_complex".into(), format!("amix=inputs={}:duration=longest:normalize=0", inputs.len()).into()]);
            let (codec, _) = codec_args(&job.profile, HwAccel::None, &ff_core::Edits::default()).map_err(|e| anyhow!(e))?;
            args.extend(codec.iter().map(OsString::from));
            duration = longest;
            hw_applied = false;
        }
    }
    args.push(path_arg(&job.output));
    Ok(Plan { args, tmp, duration, hw_applied })
}

/// Lit une ligne `-progress` (clé=valeur) et met à jour la progression ; vrai en fin de bloc.
pub fn apply_progress_line(line: &str, duration: Option<f64>, p: &mut Progress) -> bool {
    let Some((k, v)) = line.split_once('=') else { return false };
    match k.trim() {
        "out_time_us" | "out_time_ms" => {
            // out_time_ms est en réalité exprimé en microsecondes par FFmpeg.
            if let Ok(us) = v.trim().parse::<i64>() {
                p.out_time_secs = (us.max(0) as f64) / 1_000_000.0;
                if let Some(d) = duration.filter(|d| *d > 0.0) {
                    p.percent = ((p.out_time_secs / d) * 100.0).clamp(0.0, 100.0) as f32;
                }
            }
            false
        }
        "speed" => {
            if let Ok(s) = v.trim().trim_end_matches('x').trim().parse::<f32>() {
                p.speed = s;
            }
            false
        }
        "progress" => true,
        _ => false,
    }
}

/// Exécute une commande préparée.
fn exec(
    bins: &Binaries,
    plan: &Plan,
    out: &Path,
    cancel: &Arc<AtomicBool>,
    on_progress: &mut dyn FnMut(Progress),
) -> Result<()> {
    let mut child = quiet(&mut Command::new(&bins.ffmpeg))
        .args(&plan.args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("lancement de ffmpeg")?;

    // stderr lu dans un thread pour éviter tout blocage du tube ; on garde les dernières lignes.
    let stderr = child.stderr.take().ok_or_else(|| anyhow!("stderr indisponible"))?;
    let err_thread = std::thread::spawn(move || {
        let mut tail: Vec<String> = Vec::new();
        for l in BufReader::new(stderr).lines().map_while(Result::ok) {
            tail.push(l);
            if tail.len() > 20 {
                tail.remove(0);
            }
        }
        tail
    });

    let stdout = child.stdout.take().ok_or_else(|| anyhow!("stdout indisponible"))?;
    let mut p = Progress::default();
    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_file(out);
            return Err(Cancelled.into());
        }
        if apply_progress_line(&line, plan.duration, &mut p) {
            on_progress(p);
        }
    }
    let status = child.wait()?;
    let tail = err_thread.join().unwrap_or_default();
    if cancel.load(Ordering::Relaxed) {
        let _ = std::fs::remove_file(out);
        return Err(Cancelled.into());
    }
    if !status.success() {
        let _ = std::fs::remove_file(out);
        bail!("ffmpeg a échoué ({status}) :\n{}", tail.join("\n"));
    }
    p.percent = 100.0;
    on_progress(p);
    Ok(())
}

/// Exécute une tâche. `cancel` permet d'interrompre ; `on_progress` est appelé à chaque bloc ;
/// `on_info` reçoit les informations non bloquantes (repli logiciel).
pub fn run(
    bins: &Binaries,
    job: &Job,
    cancel: Arc<AtomicBool>,
    mut on_progress: impl FnMut(Progress),
    mut on_info: impl FnMut(String),
) -> Result<()> {
    // Musiques rétro (YM, SNDH, SID) : rendu WAV préalable, puis conversion normale du WAV.
    if matches!(job.kind, ff_core::JobKind::Convert) && ff_chip::is_chip(&job.input) {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let wav = std::env::temp_dir().join(format!("mediaforge_chip_{}_{nanos}.wav", std::process::id()));
        on_info(format!("Rendu audio de {}…", job.input.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()));
        ff_chip::render_to_wav(&job.input, &wav, 180.0).map_err(|e| anyhow!(e))?;
        let mut j2 = job.clone();
        j2.input = wav.clone();
        let r = run(bins, &j2, cancel, on_progress, on_info);
        let _ = std::fs::remove_file(&wav);
        return r;
    }
    let first = plan(bins, job, false)?;
    let mut result = exec(bins, &first, &job.output, &cancel, &mut on_progress);
    if let Err(e) = &result {
        if first.hw_applied && e.downcast_ref::<Cancelled>().is_none() {
            let why = format!("{e:#}").lines().last().unwrap_or("").to_string();
            on_info(format!("Accélération {} indisponible, repli logiciel ({why})", job.hw.label()));
            let soft = plan(bins, job, true)?;
            result = exec(bins, &soft, &job.output, &cancel, &mut on_progress);
            if let Some(t) = &soft.tmp {
                let _ = std::fs::remove_file(t);
            }
        }
    }
    if let Some(t) = &first.tmp {
        let _ = std::fs::remove_file(t);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn progress_parsing() {
        let mut p = Progress::default();
        apply_progress_line("out_time_us=5000000", Some(10.0), &mut p);
        apply_progress_line("speed=2.5x", Some(10.0), &mut p);
        assert!((p.percent - 50.0).abs() < 0.01);
        assert!((p.speed - 2.5).abs() < 0.01);
        assert!(apply_progress_line("progress=continue", Some(10.0), &mut p));
    }
}
