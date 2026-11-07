//! mediaforge-cli : interface en ligne de commande. v0.3.0
use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand};
use ff_core::{default_output, unique_output, Crop, Edits, HwAccel, Job};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

#[derive(Parser)]
#[command(name = "mediaforge-cli", version, about = "MediaForge - convertisseur de médias")]
struct Cli {
    /// Dossier contenant ffmpeg et ffprobe (sinon : MEDIAFORGE_FFMPEG_DIR, ./ffmpeg, PATH)
    #[arg(long, global = true)]
    ffmpeg_dir: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Vérifie la présence de FFmpeg
    Doctor,
    /// Liste les profils intégrés
    Profiles,
    /// Affiche les informations d'un média
    Info { file: PathBuf },
    /// Convertit un fichier
    Convert {
        input: PathBuf,
        /// Fichier de sortie (défaut : même nom, extension du profil)
        output: Option<PathBuf>,
        #[arg(short, long, default_value = "mp4")]
        profile: String,
        /// Début de la découpe, en secondes
        #[arg(long)]
        from: Option<f64>,
        /// Fin de la découpe, en secondes
        #[arg(long)]
        to: Option<f64>,
        /// Recadrage largeur:hauteur:x:y (pixels)
        #[arg(long)]
        crop: Option<String>,
        /// Accélération matérielle : none, nvenc, qsv, vaapi
        #[arg(long, default_value = "none")]
        hw: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Profiles => {
            for p in ff_profiles::load().all_profiles() {
                println!("{:<12} .{:<5} {}", p.id, p.extension, p.label);
            }
        }
        Cmd::Doctor => {
            let b = ff_engine::locate(cli.ffmpeg_dir.as_deref())?;
            println!("ffmpeg  : {}", b.ffmpeg.display());
            println!("ffprobe : {}", b.ffprobe.display());
            println!("version : {}", ff_engine::version(&b)?);
        }
        Cmd::Info { file } => {
            let b = ff_engine::locate(cli.ffmpeg_dir.as_deref())?;
            let i = ff_probe::probe(&b.ffprobe, &file)?;
            println!("format : {}", i.format_name);
            println!("durée  : {}", i.duration_secs.map_or("inconnue".into(), |d| format!("{d:.2} s")));
            println!("vidéo  : {}   audio : {}", i.has_video, i.has_audio);
        }
        Cmd::Convert { input, output, profile, from, to, crop, hw } => {
            let b = ff_engine::locate(cli.ffmpeg_dir.as_deref())?;
            let profile = ff_profiles::load()
                .all_profiles()
                .into_iter()
                .find(|p| p.id == profile)
                .ok_or_else(|| anyhow!("profil inconnu : {profile} (voir `profiles`)"))?;
            let output = output.unwrap_or_else(|| {
                unique_output(default_output(&input, None, &profile.extension), &input)
            });
            if output == input {
                return Err(anyhow!("la sortie ne peut pas écraser l'entrée"));
            }
            let crop = match crop {
                None => None,
                Some(c) => {
                    let v: Vec<u32> = c.split(':').filter_map(|n| n.parse().ok()).collect();
                    if v.len() != 4 {
                        return Err(anyhow!("--crop attend largeur:hauteur:x:y"));
                    }
                    Some(Crop { w: v[0], h: v[1], x: v[2], y: v[3] })
                }
            };
            let hw = match hw.as_str() {
                "none" => HwAccel::None,
                "nvenc" => HwAccel::Nvenc,
                "qsv" => HwAccel::Qsv,
                "amf" => HwAccel::Amf,
                "vaapi" => HwAccel::Vaapi,
                other => return Err(anyhow!("accélération inconnue : {other}")),
            };
            let mut job = Job::convert(input, output.clone(), profile);
            job.edits = Edits { trim_start: from, trim_end: to, crop };
            job.hw = hw;
            ff_engine::run(
                &b,
                &job,
                Arc::new(AtomicBool::new(false)),
                |p| {
                    print!("\r{:5.1} %  (x{:.1})", p.percent, p.speed);
                    let _ = std::io::stdout().flush();
                },
                |m| eprintln!("\n{m}"),
            )?;
            println!("\nTerminé : {}", output.display());
        }
    }
    Ok(())
}
