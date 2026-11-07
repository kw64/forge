//! ff-probe : analyse des médias via ffprobe (sortie JSON).
use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Default)]
pub struct MediaInfo {
    pub duration_secs: Option<f64>,
    pub has_video: bool,
    pub has_audio: bool,
    pub format_name: String,
    /// Dimensions du premier flux vidéo.
    pub size: Option<(u32, u32)>,
    /// Cadence du premier flux vidéo, telle qu'écrite par ffprobe (ex. « 30000/1001 »).
    pub fps: Option<String>,
}

#[derive(Deserialize)]
struct Raw {
    format: Option<RawFormat>,
    #[serde(default)]
    streams: Vec<RawStream>,
}
#[derive(Deserialize)]
struct RawFormat {
    duration: Option<String>,
    format_name: Option<String>,
}
#[derive(Deserialize)]
struct RawStream {
    codec_type: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    avg_frame_rate: Option<String>,
}

/// Analyse un fichier avec `ffprobe`. Le chemin est passé en argument (jamais via un shell).
pub fn probe(ffprobe: &Path, file: &Path) -> Result<MediaInfo> {
    let out = Command::new(ffprobe)
        .args(["-v", "error", "-print_format", "json", "-show_format", "-show_streams"])
        .arg(file)
        .output()
        .with_context(|| format!("impossible de lancer {}", ffprobe.display()))?;
    if !out.status.success() {
        return Err(anyhow!("ffprobe a échoué : {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    parse(&out.stdout)
}

/// Découpé pour être testable sans ffprobe.
pub fn parse(json: &[u8]) -> Result<MediaInfo> {
    let raw: Raw = serde_json::from_slice(json).context("JSON ffprobe invalide")?;
    let has = |t: &str| raw.streams.iter().any(|s| s.codec_type.as_deref() == Some(t));
    let fmt = raw.format.as_ref();
    Ok(MediaInfo {
        duration_secs: fmt.and_then(|f| f.duration.as_deref()).and_then(|d| d.parse().ok()),
        has_video: has("video"),
        has_audio: has("audio"),
        format_name: fmt.and_then(|f| f.format_name.clone()).unwrap_or_default(),
        size: raw
            .streams
            .iter()
            .find(|s| s.codec_type.as_deref() == Some("video"))
            .and_then(|s| Some((s.width?, s.height?))),
        fps: raw
            .streams
            .iter()
            .find(|s| s.codec_type.as_deref() == Some("video"))
            .and_then(|s| s.avg_frame_rate.clone())
            .filter(|f| f != "0/0" && !f.is_empty()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parse_basic() {
        let j = br#"{"format":{"duration":"12.5","format_name":"mov,mp4"},"streams":[{"codec_type":"video","width":640,"height":360},{"codec_type":"audio"}]}"#;
        let i = parse(j).unwrap();
        assert_eq!(i.duration_secs, Some(12.5));
        assert!(i.has_video && i.has_audio);
        assert_eq!(i.size, Some((640, 360)));
    }
}
