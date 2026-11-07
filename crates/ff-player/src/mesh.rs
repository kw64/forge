//! Maillages 3D (STL binaire/ASCII, OBJ) : lecture et rendu logiciel ombré. v0.7.0
use eframe::egui::{Color32, ColorImage};
use std::path::Path;

pub struct Mesh {
    /// Sommets normalisés (centrés, rayon 1).
    pub tris: Vec<[[f32; 3]; 3]>,
}

fn normalize(tris: Vec<[[f32; 3]; 3]>) -> Result<Mesh, String> {
    if tris.is_empty() {
        return Err("aucune face trouvée".into());
    }
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    for t in &tris {
        for v in t {
            for k in 0..3 {
                lo[k] = lo[k].min(v[k]);
                hi[k] = hi[k].max(v[k]);
            }
        }
    }
    let c = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0, (lo[2] + hi[2]) / 2.0];
    let r = (0..3).map(|k| (hi[k] - lo[k]) / 2.0).fold(0.0f32, |a, b| a.max(b)).max(1e-9);
    let tris = tris.into_iter().map(|t| t.map(|v| [(v[0] - c[0]) / r, (v[1] - c[1]) / r, (v[2] - c[2]) / r])).collect();
    Ok(Mesh { tris })
}

fn parse_stl(b: &[u8]) -> Result<Mesh, String> {
    let mut tris = Vec::new();
    let binary = b.len() >= 84 && {
        let n = u32::from_le_bytes([b[80], b[81], b[82], b[83]]) as usize;
        b.len() == 84 + 50 * n
    };
    if binary {
        let f = |o: usize| f32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
        let n = (b.len() - 84) / 50;
        for i in 0..n {
            let o = 84 + i * 50 + 12;
            tris.push([[f(o), f(o + 4), f(o + 8)], [f(o + 12), f(o + 16), f(o + 20)], [f(o + 24), f(o + 28), f(o + 32)]]);
        }
    } else {
        let txt = String::from_utf8_lossy(b);
        let mut v: Vec<[f32; 3]> = Vec::new();
        for l in txt.lines() {
            let mut w = l.split_whitespace();
            if w.next() == Some("vertex") {
                let p: Vec<f32> = w.filter_map(|x| x.parse().ok()).collect();
                if p.len() == 3 {
                    v.push([p[0], p[1], p[2]]);
                }
            }
        }
        tris = v.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect();
    }
    normalize(tris)
}

fn parse_obj(b: &[u8]) -> Result<Mesh, String> {
    let txt = String::from_utf8_lossy(b);
    let mut v: Vec<[f32; 3]> = Vec::new();
    let mut tris = Vec::new();
    for l in txt.lines() {
        let mut w = l.split_whitespace();
        match w.next() {
            Some("v") => {
                let p: Vec<f32> = w.take(3).filter_map(|x| x.parse().ok()).collect();
                if p.len() == 3 {
                    v.push([p[0], p[1], p[2]]);
                }
            }
            Some("f") => {
                let idx: Vec<usize> = w
                    .filter_map(|t| {
                        let i: i64 = t.split('/').next()?.parse().ok()?;
                        let k = if i < 0 { v.len() as i64 + i } else { i - 1 };
                        (k >= 0 && (k as usize) < v.len()).then_some(k as usize)
                    })
                    .collect();
                for k in 1..idx.len().saturating_sub(1) {
                    tris.push([v[idx[0]], v[idx[k]], v[idx[k + 1]]]);
                }
            }
            _ => {}
        }
    }
    normalize(tris)
}

pub fn load(p: &Path) -> Result<Mesh, String> {
    let b = std::fs::read(p).map_err(|e| e.to_string())?;
    match p.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref() {
        Some("stl") => parse_stl(&b),
        Some("obj") => parse_obj(&b),
        _ => Err("format non géré".into()),
    }
}

/// Rendu orthographique avec z-buffer et éclairage « lampe frontale » (deux faces).
pub fn render(m: &Mesh, yaw: f32, pitch: f32, zoom: f32, w: usize, h: usize, base: Color32, wire: bool) -> ColorImage {
    let (sy, cy, sp, cp) = (yaw.sin(), yaw.cos(), pitch.sin(), pitch.cos());
    let rot = |v: [f32; 3]| -> [f32; 3] {
        let x = v[0] * cy + v[2] * sy;
        let z = -v[0] * sy + v[2] * cy;
        let y = v[1] * cp - z * sp;
        let z2 = v[1] * sp + z * cp;
        [x, y, z2]
    };
    let s = zoom * 0.46 * w.min(h) as f32;
    let (cx, cyy) = (w as f32 / 2.0, h as f32 / 2.0);
    let mut px = vec![Color32::from_rgb(26, 28, 34); w * h];
    let mut zb = vec![f32::MIN; w * h];
    let proj = |v: [f32; 3]| [cx + v[0] * s, cyy - v[1] * s, v[2]];
    for t in &m.tris {
        let a = rot(t[0]);
        let b = rot(t[1]);
        let c = rot(t[2]);
        let (u, v) = ([b[0] - a[0], b[1] - a[1], b[2] - a[2]], [c[0] - a[0], c[1] - a[1], c[2] - a[2]]);
        let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
        let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        if l < 1e-12 {
            continue;
        }
        let k = 0.22 + 0.78 * (n[2] / l).abs();
        let col = Color32::from_rgb((base.r() as f32 * k) as u8, (base.g() as f32 * k) as u8, (base.b() as f32 * k) as u8);
        let (p0, p1, p2) = (proj(a), proj(b), proj(c));
        let area = (p1[0] - p0[0]) * (p2[1] - p0[1]) - (p1[1] - p0[1]) * (p2[0] - p0[0]);
        if area.abs() < 1e-9 {
            continue;
        }
        let minx = p0[0].min(p1[0]).min(p2[0]).floor().max(0.0) as usize;
        let maxx = (p0[0].max(p1[0]).max(p2[0]).ceil().min(w as f32 - 1.0)).max(0.0) as usize;
        let miny = p0[1].min(p1[1]).min(p2[1]).floor().max(0.0) as usize;
        let maxy = (p0[1].max(p1[1]).max(p2[1]).ceil().min(h as f32 - 1.0)).max(0.0) as usize;
        for y in miny..=maxy {
            for x in minx..=maxx {
                let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                let w0 = ((p1[0] - fx) * (p2[1] - fy) - (p1[1] - fy) * (p2[0] - fx)) / area;
                let w1 = ((p2[0] - fx) * (p0[1] - fy) - (p2[1] - fy) * (p0[0] - fx)) / area;
                let w2 = 1.0 - w0 - w1;
                if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                    continue;
                }
                let z = w0 * p0[2] + w1 * p1[2] + w2 * p2[2];
                let i = y * w + x;
                if z > zb[i] {
                    zb[i] = z;
                    px[i] = if wire && (w0 < 0.04 || w1 < 0.04 || w2 < 0.04) { Color32::from_rgb(20, 22, 28) } else { col };
                }
            }
        }
    }
    ColorImage { size: [w, h], source_size: eframe::egui::vec2(w as f32, h as f32), pixels: px }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn obj_quad_and_negative_index() {
        let m = parse_obj(b"v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nf 1 2 3 4\nf -1 -2 -3\n").unwrap();
        assert_eq!(m.tris.len(), 3);
    }
    #[test]
    fn stl_ascii_and_render() {
        let m = parse_stl(b"solid x\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 1 0 0\nvertex 0 1 0\nendloop\nendfacet\nendsolid").unwrap();
        assert_eq!(m.tris.len(), 1);
        let img = render(&m, 0.0, 0.0, 1.0, 64, 64, Color32::WHITE, false);
        assert!(img.pixels.iter().any(|p| p.r() > 100));
    }
    #[test]
    fn stl_binary() {
        let mut b = vec![0u8; 80];
        b.extend(1u32.to_le_bytes());
        b.extend(vec![0u8; 12]);
        for f in [0f32, 0., 0., 1., 0., 0., 0., 1., 0.] {
            b.extend(f.to_le_bytes());
        }
        b.extend([0u8; 2]);
        assert_eq!(parse_stl(&b).unwrap().tris.len(), 1);
    }
}
