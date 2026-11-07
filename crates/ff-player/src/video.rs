//! Dessin de la vidéo mpv dans le contexte OpenGL d'egui. mpv rend dans un FBO privé (texture RGBA8),
//! recopié ensuite sur la cible d'egui : mpv est ainsi isolé de l'état GL d'egui. v0.4.0
use eframe::glow::{self, HasContext};
use ff_mpv::RenderContext;
use std::sync::atomic::{AtomicBool, Ordering};

pub struct Target {
    fbo: glow::Framebuffer,
    tex: glow::Texture,
    w: i32,
    h: i32,
    /// Petite cible servant à flouter les bandes du mode cinéma.
    blur: Vec<(glow::Framebuffer, glow::Texture, i32, i32)>,
    /// (largeur, hauteur, niveaux) pour lesquels la chaîne de flou est allouée.
    blur_key: (i32, i32, usize),
}

/// Neutralise l'état GL susceptible de perturber mpv.
unsafe fn reset_state(gl: &glow::Context) {
    for cap in [glow::SCISSOR_TEST, glow::BLEND, glow::DEPTH_TEST, glow::CULL_FACE, glow::STENCIL_TEST, glow::FRAMEBUFFER_SRGB] {
        gl.disable(cap);
    }
    gl.color_mask(true, true, true, true);
    gl.depth_mask(true);
    gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 4);
    gl.pixel_store_i32(glow::UNPACK_ROW_LENGTH, 0);
    gl.use_program(None);
    gl.bind_vertex_array(None);
    gl.bind_buffer(glow::ARRAY_BUFFER, None);
    gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, None);
    for u in 0..8u32 {
        gl.active_texture(glow::TEXTURE0 + u);
        gl.bind_sampler(u, None);
        gl.bind_texture(glow::TEXTURE_2D, None);
    }
    gl.active_texture(glow::TEXTURE0);
}

unsafe fn make_target(gl: &glow::Context, w: i32, h: i32) -> Option<Target> {
    let tex = gl.create_texture().ok()?;
    gl.bind_texture(glow::TEXTURE_2D, Some(tex));
    gl.tex_image_2d(glow::TEXTURE_2D, 0, glow::RGBA8 as i32, w, h, 0, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(None));
    gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::NEAREST as i32);
    gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::NEAREST as i32);
    gl.bind_texture(glow::TEXTURE_2D, None);
    let fbo = gl.create_framebuffer().ok()?;
    gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
    gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(tex), 0);
    Some(Target { fbo, tex, w, h, blur: vec![], blur_key: (0, 0, 0) })
}

unsafe fn free_target(gl: &glow::Context, t: Target) {
    gl.delete_framebuffer(t.fbo);
    gl.delete_texture(t.tex);
    for (f, x, _, _) in t.blur {
        gl.delete_framebuffer(f);
        gl.delete_texture(x);
    }
}

/// Appelé depuis le PaintCallback : met à jour mpv si besoin, rend, puis recopie plein cadre.
/// Pipette : position demandée (pixels physiques, origine haut-gauche) et couleur lue dans l'image vidéo.
#[derive(Default)]
pub struct Probe {
    pub req: std::sync::Mutex<Option<(f32, f32)>>,
    pub res: std::sync::Mutex<Option<[u8; 4]>>,
}

pub fn draw(painter: &eframe::egui_glow::Painter, size_px: [u32; 2], rc: &RenderContext, dirty: &AtomicBool, target: &mut Option<Target>, cinema: f32, blur: u32, probe: &Probe) {
    let gl = painter.gl();
    let (w, h) = (size_px[0].max(1) as i32, size_px[1].max(1) as i32);
    unsafe {
        reset_state(gl);
        if target.as_ref().map(|t| (t.w, t.h)) != Some((w, h)) {
            if let Some(t) = target.take() {
                free_target(gl, t);
            }
            *target = make_target(gl, w, h);
        }
        let Some(t) = target.as_mut() else { return };
        if dirty.swap(false, Ordering::Relaxed) {
            rc.update();
        }
        // flip_y : l'image doit être à l'endroit dans les coordonnées GL (origine en bas).
        let _ = rc.render(t.fbo.0.get() as i32, w, h, true);
        // Recopie vers la cible d'egui (framebuffer par défaut ou FBO intermédiaire).
        gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(t.fbo));
        gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, painter.intermediate_fbo());
        gl.blit_framebuffer(0, 0, w, h, 0, 0, w, h, glow::COLOR_BUFFER_BIT, glow::NEAREST);
        // Pipette : lecture d'un pixel de la vidéo seule (sans interface) dans le FBO de rendu.
        if let Some((px, py)) = probe.req.lock().ok().and_then(|g| *g) {
            let (x, y) = ((px as i32).clamp(0, w - 1), (h - 1 - py as i32).clamp(0, h - 1));
            let mut buf = [0u8; 4];
            gl.pixel_store_i32(glow::PACK_ALIGNMENT, 1);
            gl.read_pixels(x, y, 1, 1, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelPackData::Slice(Some(&mut buf)));
            if let Ok(mut r) = probe.res.lock() {
                *r = Some(buf);
            }
        }
        if cinema > 0.0 {
            cinema_fill(gl, t, painter.intermediate_fbo(), cinema, blur);
        }
        gl.bind_framebuffer(glow::FRAMEBUFFER, painter.intermediate_fbo());
    }
}

/// Mode cinéma : les 10 % de bord de la vidéo sont recopiés en miroir, étirés et flous dans les bandes noires.
/// (blit à coordonnées inversées = miroir ; réductions successives puis agrandissements filtrés = flou doux, sans shader.)
/// `blur` : 0 = aucun, 1..=6 = de plus en plus flou.
unsafe fn cinema_fill(gl: &glow::Context, t: &mut Target, screen: Option<glow::Framebuffer>, aspect: f32, blur: u32) {
    let (w, h) = (t.w, t.h);
    let sa = w as f32 / h as f32;
    if (sa - aspect).abs() < 0.02 {
        return;
    }
    let levels = blur.min(6) as usize;
    // Recouvrement : la bande mord de quelques pixels sur la vidéo pour ne jamais laisser de liseré noir
    // (l'arrondi de mpv peut différer d'un pixel de notre calcul).
    const OV: i32 = 3;
    // (zone source en miroir, bande destination)
    let mut jobs: Vec<([i32; 4], [i32; 4])> = vec![];
    if sa > aspect {
        let vw = (h as f32 * aspect).round() as i32;
        let x0 = (w - vw) / 2;
        let x1 = w - x0 - vw; // largeur de la bande droite (peut différer de 1 px)
        let s = ((vw as f32 * 0.10) as i32).max(2);
        jobs.push(([x0 + OV + s, 0, x0 + OV, h], [0, 0, x0 + OV, h]));
        jobs.push(([w - x1 - OV, 0, w - x1 - OV - s, h], [w - x1 - OV, 0, w, h]));
    } else {
        let vh = (w as f32 / aspect).round() as i32;
        let y0 = (h - vh) / 2;
        let y1 = h - y0 - vh;
        let s = ((vh as f32 * 0.10) as i32).max(2);
        jobs.push(([0, y0 + OV + s, w, y0 + OV], [0, 0, w, y0 + OV]));
        jobs.push(([0, h - y1 - OV, w, h - y1 - OV - s], [0, h - y1 - OV, w, h]));
    }
    // Les deux bandes ont exactement la même taille (la plus grande) : les cibles réduites sont alors
    // allouées à la taille juste, sans texels inutilisés qui assombriraient les bords au filtrage.
    let (cw, ch) = jobs.iter().fold((0, 0), |m, (_, d)| (m.0.max(d[2] - d[0]), m.1.max(d[3] - d[1])));
    if sa > aspect {
        jobs[0].1 = [0, 0, cw, h];
        jobs[1].1 = [w - cw, 0, w, h];
    } else {
        jobs[0].1 = [0, 0, w, ch];
        jobs[1].1 = [0, h - ch, w, h];
    }
    let (cw, ch) = (jobs[0].1[2] - jobs[0].1[0], jobs[0].1[3] - jobs[0].1[1]);
    if cw < 1 || ch < 1 {
        return;
    }
    if t.blur_key != (cw, ch, levels) {
        for (f, x, _, _) in t.blur.drain(..) {
            gl.delete_framebuffer(f);
            gl.delete_texture(x);
        }
        for k in 1..=levels as i32 {
            let (bw, bh) = ((cw >> k).max(1), (ch >> k).max(1));
            let (Ok(tex), Ok(fbo)) = (gl.create_texture(), gl.create_framebuffer()) else { return };
            gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            gl.tex_image_2d(glow::TEXTURE_2D, 0, glow::RGBA8 as i32, bw, bh, 0, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(None));
            gl.bind_texture(glow::TEXTURE_2D, None);
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(tex), 0);
            t.blur.push((fbo, tex, bw, bh));
        }
        t.blur_key = (cw, ch, levels);
    }
    for (src, dst) in jobs {
        let (dw, dh) = (dst[2] - dst[0], dst[3] - dst[1]);
        if dw < 1 || dh < 1 {
            continue;
        }
        // Taille de la bande à chaque niveau (1 = moitié, 2 = quart…).
        let sz = |k: usize| ((dw >> k).max(1), (dh >> k).max(1));
        gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(t.fbo));
        if levels == 0 {
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, screen);
            gl.blit_framebuffer(src[0], src[1], src[2], src[3], dst[0], dst[1], dst[2], dst[3], glow::COLOR_BUFFER_BIT, glow::LINEAR);
            continue;
        }
        // Descente : source -> 1/2 -> 1/4 -> …
        let (w1, h1) = sz(1);
        gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(t.blur[0].0));
        gl.blit_framebuffer(src[0], src[1], src[2], src[3], 0, 0, w1, h1, glow::COLOR_BUFFER_BIT, glow::LINEAR);
        for k in 2..=levels {
            let (pw, ph) = sz(k - 1);
            let (cw, ch) = sz(k);
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(t.blur[k - 2].0));
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(t.blur[k - 1].0));
            gl.blit_framebuffer(0, 0, pw, ph, 0, 0, cw, ch, glow::COLOR_BUFFER_BIT, glow::LINEAR);
        }
        // Remontée : … -> 1/4 -> 1/2 -> bande (l'agrandissement bilinéaire répété adoucit les carrés).
        for k in (2..=levels).rev() {
            let (cw, ch) = sz(k);
            let (pw, ph) = sz(k - 1);
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(t.blur[k - 1].0));
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(t.blur[k - 2].0));
            gl.blit_framebuffer(0, 0, cw, ch, 0, 0, pw, ph, glow::COLOR_BUFFER_BIT, glow::LINEAR);
        }
        gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(t.blur[0].0));
        gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, screen);
        gl.blit_framebuffer(0, 0, w1, h1, dst[0], dst[1], dst[2], dst[3], glow::COLOR_BUFFER_BIT, glow::LINEAR);
    }
}
