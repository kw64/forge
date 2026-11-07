//! Rendu audio des musiques rétro en WAV 16 bits stéréo. v0.1.0
//!
//! - **Atari ST** : `.ym` (YM2/3/5/6, YMT) et `.sndh` (émulation 68000 + YM2149) — bibliothèques `ym2149-*`.
//! - **Commodore 64** : `.sid` (PSID v1-v4) — processeur 6502 + SID (reSID). Les RSID (qui exigent un C64 complet) sont refusés.
//! - **Amiga / PC (trackers)** : `.mod .xm .s3m .it …` sont lus directement par FFmpeg (libopenmpt) ; voir `TRACKER_EXT`.
use std::path::Path;

pub mod sid;

/// Extensions rendues par ce crate (rendu WAV préalable).
pub const CHIP_EXT: &[&str] = &["ym", "sndh", "sid"];

/// Extensions de modules tracker lues nativement par FFmpeg/mpv (libopenmpt).
pub const TRACKER_EXT: &[&str] = &[
    "mod", "xm", "s3m", "it", "mtm", "669", "stm", "okt", "med", "far", "ult", "amf", "dbm", "dsm", "gdm", "imf", "j2b", "mdl", "ptm", "psm", "umx", "mo3", "mmd0", "mmd1", "mmd2", "mmd3", "digi", "dmf", "dtm", "ice", "mt2", "plm", "sfx", "stp", "symmod",
];

pub const SAMPLE_RATE: u32 = 44_100;

/// Fichier à rendre via ce crate ?
pub fn is_chip(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).map(|e| CHIP_EXT.contains(&e.to_ascii_lowercase().as_str())).unwrap_or(false)
}

/// Audio PCM entrelacé gauche/droite.
pub struct Pcm {
    pub rate: u32,
    pub samples: Vec<i16>,
}

fn f32_to_i16(x: f32) -> i16 {
    (x.clamp(-1.0, 1.0) * 32767.0) as i16
}

/// Rend le fichier (max `max_secs` secondes). Renvoie une erreur lisible si le format n'est pas pris en charge.
pub fn render(path: &Path, max_secs: f32) -> Result<Pcm, String> {
    let data = std::fs::read(path).map_err(|e| format!("{} : {e}", path.display()))?;
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let max = (max_secs.max(1.0) * SAMPLE_RATE as f32) as usize;
    match ext.as_str() {
        "ym" => render_ym(&data, max),
        "sndh" => render_sndh(&data, max),
        "sid" => sid::render(&data, max_secs),
        _ => Err(format!("format non géré : .{ext}")),
    }
}

fn render_ym(data: &[u8], max: usize) -> Result<Pcm, String> {
    use ym2149_common::ChiptunePlayerBase;
    use ym2149_ym_replayer::load_song_with_rate;
    let (mut p, sum) = load_song_with_rate(data, SAMPLE_RATE).map_err(|e| format!("YM : {e}"))?;
    p.play();
    let total = sum.total_samples().min(max).max(1);
    let mono = p.generate_samples(total);
    let mut samples = Vec::with_capacity(total * 2);
    for s in mono {
        let v = f32_to_i16(s);
        samples.push(v);
        samples.push(v);
    }
    Ok(Pcm { rate: SAMPLE_RATE, samples })
}

fn render_sndh(data: &[u8], max: usize) -> Result<Pcm, String> {
    use ym2149_common::ChiptunePlayerBase;
    use ym2149_sndh_replayer::load_sndh;
    let mut p = load_sndh(data, SAMPLE_RATE).map_err(|e| format!("SNDH : {e}"))?;
    let sub = p.default_subsong().max(1);
    p.init_subsong(sub).map_err(|e| format!("SNDH : {e}"))?;
    p.play();
    // Durée annoncée par le fichier si elle existe, sinon la limite demandée.
    let want = p
        .get_subsong_info(sub)
        .filter(|i| i.player_tick_count > 0 && i.player_tick_rate > 0)
        .map(|i| (i.player_tick_count as f64 / i.player_tick_rate as f64 * SAMPLE_RATE as f64) as usize)
        .unwrap_or(max)
        .min(max)
        .max(1);
    let mut out = Vec::with_capacity(want * 2);
    let mut buf = vec![0.0f32; 2 * 4096];
    while out.len() < want * 2 {
        let n = ((want * 2 - out.len()).min(buf.len()) / 2) * 2;
        let lc = p.render_f32_stereo(&mut buf[..n]);
        out.extend(buf[..n].iter().map(|s| f32_to_i16(*s)));
        if lc >= 1 && want == max {
            break; // un tour complet suffit quand la durée n'est pas connue
        }
    }
    Ok(Pcm { rate: SAMPLE_RATE, samples: out })
}

/// Écrit un WAV PCM 16 bits.
pub fn write_wav(path: &Path, pcm: &Pcm) -> std::io::Result<()> {
    use std::io::Write;
    let data_len = (pcm.samples.len() * 2) as u32;
    let mut f = std::io::BufWriter::new(std::fs::File::create(path)?);
    f.write_all(b"RIFF")?;
    f.write_all(&(36 + data_len).to_le_bytes())?;
    f.write_all(b"WAVEfmt ")?;
    f.write_all(&16u32.to_le_bytes())?;
    f.write_all(&1u16.to_le_bytes())?; // PCM
    f.write_all(&2u16.to_le_bytes())?; // stéréo
    f.write_all(&pcm.rate.to_le_bytes())?;
    f.write_all(&(pcm.rate * 4).to_le_bytes())?;
    f.write_all(&4u16.to_le_bytes())?;
    f.write_all(&16u16.to_le_bytes())?;
    f.write_all(b"data")?;
    f.write_all(&data_len.to_le_bytes())?;
    for s in &pcm.samples {
        f.write_all(&s.to_le_bytes())?;
    }
    f.flush()
}

/// Rend `src` dans un WAV temporaire et renvoie son chemin.
pub fn render_to_wav(src: &Path, dst: &Path, max_secs: f32) -> Result<(), String> {
    let pcm = render(src, max_secs)?;
    if pcm.samples.is_empty() {
        return Err("rendu vide".into());
    }
    write_wav(dst, &pcm).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ym3_minimal() {
        // YM3 : "YM3!" puis 14 registres × N images, rangés par registre.
        let n = 100usize;
        let mut d = b"YM3!".to_vec();
        for r in 0..14 {
            for _ in 0..n {
                d.push(match r {
                    0 => 0x40,  // période voie A (bas)
                    7 => 0x3E,  // mixeur : tonalité A
                    8 => 0x0F,  // volume A
                    _ => 0,
                });
            }
        }
        let pcm = render_ym(&d, 44_100 * 5).unwrap();
        assert!(!pcm.samples.is_empty());
        assert!(pcm.samples.iter().map(|s| s.abs()).max().unwrap() > 500);
    }

    #[test]
    fn wav_header() {
        let dir = std::env::temp_dir().join("ffchip_t.wav");
        write_wav(&dir, &Pcm { rate: 44100, samples: vec![0, 1, 2, 3] }).unwrap();
        let b = std::fs::read(&dir).unwrap();
        assert_eq!(&b[0..4], b"RIFF");
        assert_eq!(b.len(), 44 + 8);
        let _ = std::fs::remove_file(dir);
    }
}
