//! Traitement d'images par lots : redimensionnement (px/cm/pouces, côté long/court, mégapixels, %),
//! conversion de format et politique d'écrasement sûre (fichier temporaire puis renommage). v0.8.0
use image::{imageops::FilterType, DynamicImage, ImageDecoder, ImageFormat, ImageReader};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Unit {
    Px,
    Cm,
    Inch,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Resize {
    /// Aucun redimensionnement (conversion seule).
    None,
    /// Largeur et/ou hauteur (0 = non imposée). Avec `keep_ratio`, l'image tient dans le cadre.
    Sides { w: f64, h: f64, keep_ratio: bool },
    Long(f64),
    Short(f64),
    Megapixels(f64),
    Percent { w: f64, h: f64 },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OutFormat {
    Same,
    Jpeg,
    Png,
    WebP,
    Bmp,
    Tiff,
}

impl OutFormat {
    pub const ALL: [OutFormat; 6] = [OutFormat::Same, OutFormat::Jpeg, OutFormat::Png, OutFormat::WebP, OutFormat::Bmp, OutFormat::Tiff];
    pub fn label(self) -> &'static str {
        match self {
            OutFormat::Same => "Format d'origine",
            OutFormat::Jpeg => "JPEG",
            OutFormat::Png => "PNG",
            OutFormat::WebP => "WebP",
            OutFormat::Bmp => "BMP",
            OutFormat::Tiff => "TIFF",
        }
    }
    fn ext(self) -> Option<&'static str> {
        Some(match self {
            OutFormat::Same => return None,
            OutFormat::Jpeg => "jpg",
            OutFormat::Png => "png",
            OutFormat::WebP => "webp",
            OutFormat::Bmp => "bmp",
            OutFormat::Tiff => "tif",
        })
    }
}

/// Que faire si le fichier de sortie existe déjà.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Overwrite {
    /// Écrase (avec `backup` : l'ancien fichier devient `.bak`).
    Replace,
    /// Ne touche à rien.
    Skip,
    /// Crée « nom (1).ext ».
    Rename,
}

#[derive(Clone, Debug)]
pub struct Options {
    pub resize: Resize,
    pub unit: Unit,
    pub dpi: f64,
    pub no_upscale: bool,
    pub format: OutFormat,
    pub quality: u8,
    pub overwrite: Overwrite,
    pub backup: bool,
    pub out_dir: Option<PathBuf>,
    pub suffix: String,
}

impl Default for Options {
    fn default() -> Self {
        Options { resize: Resize::None, unit: Unit::Px, dpi: 300.0, no_upscale: true, format: OutFormat::Same, quality: 90, overwrite: Overwrite::Rename, backup: false, out_dir: None, suffix: String::new() }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Written(PathBuf),
    Skipped,
}

fn to_px(v: f64, unit: Unit, dpi: f64) -> f64 {
    match unit {
        Unit::Px => v,
        Unit::Inch => v * dpi,
        Unit::Cm => v / 2.54 * dpi,
    }
}

/// Taille de sortie pour une image de `w`×`h` pixels.
pub fn target_size(o: &Options, w: u32, h: u32) -> (u32, u32) {
    let (fw, fh) = (w as f64, h as f64);
    let px = |v: f64| to_px(v, o.unit, o.dpi);
    let (tw, th) = match o.resize {
        Resize::None => (fw, fh),
        Resize::Sides { w: ww, h: hh, keep_ratio } => match (ww > 0.0, hh > 0.0) {
            (true, true) if !keep_ratio => (px(ww), px(hh)),
            (true, true) => {
                let s = (px(ww) / fw).min(px(hh) / fh);
                (fw * s, fh * s)
            }
            (true, false) => (px(ww), fh * px(ww) / fw),
            (false, true) => (fw * px(hh) / fh, px(hh)),
            _ => (fw, fh),
        },
        Resize::Long(v) => {
            let s = px(v) / fw.max(fh);
            (fw * s, fh * s)
        }
        Resize::Short(v) => {
            let s = px(v) / fw.min(fh);
            (fw * s, fh * s)
        }
        Resize::Megapixels(mp) => {
            let s = (mp * 1_000_000.0 / (fw * fh)).sqrt();
            (fw * s, fh * s)
        }
        Resize::Percent { w: pw, h: ph } => (fw * pw / 100.0, fh * ph / 100.0),
    };
    let (mut tw, mut th) = (tw.round().max(1.0), th.round().max(1.0));
    if o.no_upscale && (tw > fw || th > fh) && !matches!(o.resize, Resize::Sides { keep_ratio: false, .. } | Resize::Percent { .. }) {
        let s = (fw / tw).min(fh / th).min(1.0);
        tw = (tw * s).round().max(1.0);
        th = (th * s).round().max(1.0);
    }
    (tw as u32, th as u32)
}

fn format_of(p: &Path) -> Option<ImageFormat> {
    ImageFormat::from_path(p).ok()
}

/// Nom « base (1).ext » libre.
pub fn unique_path(p: &Path) -> PathBuf {
    if !p.exists() {
        return p.to_path_buf();
    }
    let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = p.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    (1..100_000).map(|n| p.with_file_name(format!("{stem} ({n}){ext}"))).find(|c| !c.exists()).unwrap_or_else(|| p.to_path_buf())
}

/// Chemin de sortie théorique (avant application de la politique d'écrasement).
pub fn planned_output(input: &Path, o: &Options) -> PathBuf {
    let dir = o.out_dir.clone().or_else(|| input.parent().map(Path::to_path_buf)).unwrap_or_default();
    let stem = input.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "image".into());
    let ext = o.format.ext().map(String::from).or_else(|| input.extension().map(|e| e.to_string_lossy().to_ascii_lowercase())).unwrap_or_else(|| "png".into());
    dir.join(format!("{stem}{}.{ext}", o.suffix))
}

/// Vrai si l'opération remplacera un fichier existant.
pub fn would_replace(input: &Path, o: &Options) -> bool {
    o.overwrite == Overwrite::Replace && planned_output(input, o).exists()
}

fn encode(img: &DynamicImage, out: &Path, fmt: ImageFormat, quality: u8) -> Result<(), String> {
    use image::codecs::jpeg::JpegEncoder;
    match fmt {
        ImageFormat::Jpeg => {
            let f = std::fs::File::create(out).map_err(|e| e.to_string())?;
            let enc = JpegEncoder::new_with_quality(std::io::BufWriter::new(f), quality.clamp(1, 100));
            img.to_rgb8().write_with_encoder(enc).map_err(|e| e.to_string())
        }
        ImageFormat::Bmp | ImageFormat::Gif => img.save_with_format(out, fmt).map_err(|e| e.to_string()),
        _ => img.save_with_format(out, fmt).map_err(|e| e.to_string()),
    }
}

/// Traite une image : lecture (orientation EXIF appliquée), redimensionnement, écriture sûre.
pub fn process(input: &Path, o: &Options) -> Result<Outcome, String> {
    let planned = planned_output(input, o);
    let same_file = planned == input;
    let out = if planned.exists() {
        match o.overwrite {
            Overwrite::Skip => return Ok(Outcome::Skipped),
            Overwrite::Rename => unique_path(&planned),
            Overwrite::Replace => planned.clone(),
        }
    } else {
        planned.clone()
    };
    let mut dec = ImageReader::open(input).map_err(|e| e.to_string())?.with_guessed_format().map_err(|e| e.to_string())?.into_decoder().map_err(|e| e.to_string())?;
    let orient = dec.orientation().map_err(|e| e.to_string())?;
    let mut img = DynamicImage::from_decoder(dec).map_err(|e| e.to_string())?;
    img.apply_orientation(orient);
    let (tw, th) = target_size(o, img.width(), img.height());
    if (tw, th) != (img.width(), img.height()) {
        img = img.resize_exact(tw, th, FilterType::Lanczos3);
    }
    let fmt = format_of(&out).ok_or("format de sortie inconnu")?;
    if let Some(d) = out.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    // Écriture dans un fichier temporaire voisin, puis renommage : jamais de fichier à moitié écrit.
    let tmp = out.with_file_name(format!(".ff_tmp_{}_{}", std::process::id(), out.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
    if let Err(e) = encode(&img, &tmp, fmt, o.quality) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    if out.exists() && o.overwrite == Overwrite::Replace {
        if o.backup {
            let bak = PathBuf::from(format!("{}.bak", out.display()));
            let _ = std::fs::remove_file(&bak);
            std::fs::rename(&out, &bak).map_err(|e| format!("sauvegarde : {e}"))?;
        } else if !same_file {
            // le renommage ci-dessous remplace le fichier existant
        }
    }
    std::fs::rename(&tmp, &out).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })?;
    Ok(Outcome::Written(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(r: Resize) -> Options {
        Options { resize: r, ..Default::default() }
    }

    #[test]
    fn sizes() {
        assert_eq!(target_size(&opts(Resize::Sides { w: 500.0, h: 0.0, keep_ratio: true }), 1000, 500), (500, 250));
        assert_eq!(target_size(&opts(Resize::Sides { w: 400.0, h: 400.0, keep_ratio: true }), 1000, 500), (400, 200));
        assert_eq!(target_size(&opts(Resize::Long(200.0)), 1000, 500), (200, 100));
        assert_eq!(target_size(&opts(Resize::Short(100.0)), 1000, 500), (200, 100));
        assert_eq!(target_size(&opts(Resize::Percent { w: 50.0, h: 50.0 }), 1000, 500), (500, 250));
        let mp = target_size(&opts(Resize::Megapixels(0.5)), 2000, 1000);
        assert!((mp.0 as f64 * mp.1 as f64 - 500_000.0).abs() < 4000.0);
        // pas d'agrandissement
        assert_eq!(target_size(&opts(Resize::Long(5000.0)), 1000, 500), (1000, 500));
        let mut up = opts(Resize::Long(2000.0));
        up.no_upscale = false;
        assert_eq!(target_size(&up, 1000, 500), (2000, 1000));
        // 10 cm à 254 dpi = 1000 px
        let mut cm = opts(Resize::Long(10.0));
        cm.unit = Unit::Cm;
        cm.dpi = 254.0;
        cm.no_upscale = false;
        assert_eq!(target_size(&cm, 500, 250), (1000, 500));
    }

    #[test]
    fn overwrite_policies() {
        let d = std::env::temp_dir().join(format!("ffimg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let src = d.join("a.png");
        DynamicImage::new_rgb8(400, 200).save(&src).unwrap();
        let mut o = opts(Resize::Long(100.0));
        // Rename : l'original reste, nouvelle copie « a (1).png »
        let r = process(&src, &o).unwrap();
        assert_eq!(r, Outcome::Written(d.join("a (1).png")));
        assert_eq!(image::open(&src).unwrap().width(), 400);
        assert_eq!(image::open(d.join("a (1).png")).unwrap().width(), 100);
        // Skip
        o.overwrite = Overwrite::Skip;
        assert_eq!(process(&src, &o).unwrap(), Outcome::Skipped);
        // Replace avec sauvegarde
        o.overwrite = Overwrite::Replace;
        o.backup = true;
        assert!(would_replace(&src, &o));
        process(&src, &o).unwrap();
        assert_eq!(image::open(&src).unwrap().width(), 100);
        assert_eq!(ImageReader::open(d.join("a.png.bak")).unwrap().with_guessed_format().unwrap().decode().unwrap().width(), 400);
        // Conversion de format
        let mut j = opts(Resize::None);
        j.format = OutFormat::Jpeg;
        let r = process(&src, &j).unwrap();
        assert_eq!(r, Outcome::Written(d.join("a.jpg")));
        let _ = std::fs::remove_dir_all(&d);
    }
}
