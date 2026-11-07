//! Rendu des documents (PDF, AI, PSD, EPS) en PNG pour mpv. v0.6.0
//! PDF/AI : hayro (Rust pur) ; PSD : crate psd (image aplatie) ; EPS (et AI non PDF) : Ghostscript s'il est installé.
use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{PixmapSettings, RenderCache, RenderSettings};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

/// Côté le plus long visé, en pixels.
const TARGET: f32 = 2400.0;

fn ext(p: &Path) -> String {
    p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase()
}

fn pdf_open(p: &Path) -> Option<Pdf> {
    Pdf::new(Arc::new(std::fs::read(p).ok()?)).ok()
}

/// Nombre de pages (1 pour les formats non paginés).
pub fn page_count(p: &Path) -> usize {
    match ext(p).as_str() {
        "pdf" | "ai" => pdf_open(p).map(|d| d.pages().len().max(1)).unwrap_or(1),
        _ => 1,
    }
}

fn cache_png(p: &Path, page: usize) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    p.hash(&mut h);
    std::fs::metadata(p).and_then(|m| m.modified()).ok().hash(&mut h);
    let d = std::env::temp_dir().join("mediaforge-doc");
    let _ = std::fs::create_dir_all(&d);
    d.join(format!("{:016x}-{page}.png", h.finish()))
}

fn render_pdf(p: &Path, page: usize, out: &Path) -> Result<(), String> {
    let pdf = pdf_open(p).ok_or("PDF illisible ou protégé")?;
    let pages = pdf.pages();
    let pg = pages.get(page).ok_or("page inexistante")?;
    let (w, h) = pg.render_dimensions();
    let scale = (TARGET / w.max(h).max(1.0)).clamp(0.5, 4.0);
    let cache = RenderCache::new();
    let px = hayro::render(
        pg,
        &cache,
        &InterpreterSettings::default(),
        &RenderSettings::default(),
        &PixmapSettings { x_scale: scale, y_scale: scale, bg_color: WHITE },
    );
    let png = px.into_png().map_err(|e| e.to_string())?;
    std::fs::write(out, png).map_err(|e| e.to_string())
}

fn render_psd(p: &Path, out: &Path) -> Result<(), String> {
    let bytes = std::fs::read(p).map_err(|e| e.to_string())?;
    let psd = psd::Psd::from_bytes(&bytes).map_err(|e| e.to_string())?;
    let img = image::RgbaImage::from_raw(psd.width(), psd.height(), psd.rgba()).ok_or("PSD : dimensions invalides")?;
    img.save_with_format(out, image::ImageFormat::Png).map_err(|e| e.to_string())
}

fn render_gs(p: &Path, page: usize, out: &Path) -> Result<(), String> {
    let names: &[&str] = if cfg!(windows) { &["gswin64c", "gswin32c", "gs"] } else { &["gs"] };
    for n in names {
        let r = Command::new(n)
            .args(["-dSAFER", "-dBATCH", "-dNOPAUSE", "-dQUIET", "-dEPSCrop", "-sDEVICE=png16m", "-r150", "-dTextAlphaBits=4", "-dGraphicsAlphaBits=4"])
            .arg(format!("-dFirstPage={}", page + 1))
            .arg(format!("-dLastPage={}", page + 1))
            .arg(format!("-sOutputFile={}", out.display()))
            .arg(p)
            .output();
        if let Ok(o) = r {
            if o.status.success() && out.exists() {
                return Ok(());
            }
            return Err(String::from_utf8_lossy(&o.stderr).trim().to_string());
        }
    }
    Err("Ghostscript introuvable (requis pour l'EPS)".into())
}

/// Rend la page `page` (0-based) dans le cache et retourne le PNG.
pub fn render(p: &Path, page: usize) -> Result<PathBuf, String> {
    let out = cache_png(p, page);
    if out.exists() {
        return Ok(out);
    }
    let r = match ext(p).as_str() {
        "pdf" => render_pdf(p, page, &out).or_else(|_| render_gs(p, page, &out)),
        "ai" => render_pdf(p, page, &out).or_else(|_| render_gs(p, page, &out)),
        "psd" => render_psd(p, &out),
        "eps" => render_gs(p, page, &out),
        _ => Err("format non géré".into()),
    };
    r.map(|_| out)
}
