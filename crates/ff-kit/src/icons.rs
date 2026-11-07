//! Icônes dessinées au pinceau egui : aucune dépendance à une police de symboles (Windows/Linux identiques). v0.4.0
use egui::{self, pos2, vec2, Color32, Painter, Pos2, Rect, Response, Sense, Shape, Stroke, Ui};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Prev,
    Next,
    Play,
    Pause,
    Volume,
    Mute,
    Fullscreen,
    Windowed,
    List,
    Shuffle,
    Repeat,
    Camera,
    Subs,
    Rotate,
    Fit,
    ZoomIn,
    ZoomOut,
    Copy,
    Slideshow,
    Plus,
    Close,
    Note,
    Film,
    Picture,
    Folder,
    Gear,
    Tools,
    Stop,
    Trash,
    Scissors,
    Sun,
    Moon,
    Doc,
    Sliders,
    Check,
    Pencil,
    Palette,
    Merge,
    Grid,
    Clock,
}

fn tri(p: &Painter, pts: [Pos2; 3], c: Color32) {
    p.add(Shape::convex_polygon(pts.to_vec(), c, Stroke::NONE));
}

/// Dessine `ic` dans `r` (carré).
pub fn paint(p: &Painter, r: Rect, ic: Icon, c: Color32) {
    let (cx, cy, s) = (r.center().x, r.center().y, r.width().min(r.height()) / 2.0);
    let st = Stroke::new((s / 7.0).max(1.4), c);
    let fr = |x0: f32, y0: f32, x1: f32, y1: f32| Rect::from_min_max(pos2(cx + x0 * s, cy + y0 * s), pos2(cx + x1 * s, cy + y1 * s));
    let pt = |x: f32, y: f32| pos2(cx + x * s, cy + y * s);
    let line = |a: Pos2, b: Pos2| {
        p.line_segment([a, b], st);
    };
    match ic {
        Icon::Play => tri(p, [pt(-0.55, -0.7), pt(-0.55, 0.7), pt(0.7, 0.0)], c),
        Icon::Pause => {
            p.rect_filled(fr(-0.6, -0.65, -0.15, 0.65), 1.0, c);
            p.rect_filled(fr(0.15, -0.65, 0.6, 0.65), 1.0, c);
        }
        Icon::Prev => {
            p.rect_filled(fr(-0.7, -0.6, -0.4, 0.6), 1.0, c);
            tri(p, [pt(0.7, -0.65), pt(0.7, 0.65), pt(-0.3, 0.0)], c);
        }
        Icon::Next => {
            p.rect_filled(fr(0.4, -0.6, 0.7, 0.6), 1.0, c);
            tri(p, [pt(-0.7, -0.65), pt(-0.7, 0.65), pt(0.3, 0.0)], c);
        }
        Icon::Volume | Icon::Mute => {
            p.rect_filled(fr(-0.8, -0.28, -0.4, 0.28), 0.0, c);
            tri(p, [pt(-0.4, -0.28), pt(-0.4, 0.28), pt(0.05, 0.0)], c);
            p.add(Shape::convex_polygon(vec![pt(-0.4, -0.28), pt(0.1, -0.7), pt(0.1, 0.7), pt(-0.4, 0.28)], c, Stroke::NONE));
            if ic == Icon::Volume {
                for k in [0.45f32, 0.8] {
                    let pts: Vec<Pos2> = (-4..=4).map(|i| {
                        let a = i as f32 * 0.17;
                        pt(0.1 + k * a.cos() * 0.8, k * a.sin() * 0.9)
                    }).collect();
                    p.add(Shape::line(pts, st));
                }
            } else {
                line(pt(0.3, -0.35), pt(0.85, 0.35));
                line(pt(0.3, 0.35), pt(0.85, -0.35));
            }
        }
        Icon::Fullscreen | Icon::Windowed => {
            let k = if ic == Icon::Fullscreen { 1.0 } else { -1.0 };
            for (sx, sy) in [(-1.0f32, -1.0f32), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                let (ax, ay) = (sx * (0.8 - 0.0 * k), sy * 0.8);
                let (bx, by) = (sx * (0.8 - 0.5 * k), sy * 0.8);
                let (dx, dy) = (sx * 0.8, sy * (0.8 - 0.5 * k));
                if ic == Icon::Fullscreen {
                    line(pt(ax, ay), pt(bx, by));
                    line(pt(ax, ay), pt(dx, dy));
                } else {
                    let (ix, iy) = (sx * 0.3, sy * 0.3);
                    line(pt(ix, iy), pt(sx * 0.8, iy));
                    line(pt(ix, iy), pt(ix, sy * 0.8));
                }
            }
        }
        Icon::List => {
            for y in [-0.55f32, 0.0, 0.55] {
                p.circle_filled(pt(-0.7, y), s * 0.1, c);
                line(pt(-0.35, y), pt(0.8, y));
            }
        }
        Icon::Shuffle => {
            line(pt(-0.8, -0.5), pt(-0.2, -0.5));
            line(pt(-0.2, -0.5), pt(0.3, 0.5));
            line(pt(0.3, 0.5), pt(0.55, 0.5));
            line(pt(-0.8, 0.5), pt(-0.2, 0.5));
            line(pt(-0.2, 0.5), pt(0.3, -0.5));
            line(pt(0.3, -0.5), pt(0.55, -0.5));
            tri(p, [pt(0.5, -0.8), pt(0.5, -0.2), pt(0.9, -0.5)], c);
            tri(p, [pt(0.5, 0.2), pt(0.5, 0.8), pt(0.9, 0.5)], c);
        }
        Icon::Repeat => {
            line(pt(-0.7, -0.3), pt(-0.7, 0.4));
            line(pt(-0.7, 0.4), pt(0.3, 0.4));
            line(pt(0.7, 0.3), pt(0.7, -0.4));
            line(pt(0.7, -0.4), pt(-0.3, -0.4));
            tri(p, [pt(-0.45, -0.75), pt(-0.45, -0.05), pt(0.0, -0.4)], c);
            tri(p, [pt(0.45, 0.05), pt(0.45, 0.75), pt(0.0, 0.4)], c);
        }
        Icon::Camera => {
            p.rect_stroke(fr(-0.85, -0.45, 0.85, 0.65), 2.0, st, egui::StrokeKind::Inside);
            p.rect_filled(fr(-0.35, -0.7, 0.2, -0.45), 1.0, c);
            p.circle_stroke(pt(0.0, 0.1), s * 0.3, st);
        }
        Icon::Subs => {
            p.rect_stroke(fr(-0.85, -0.6, 0.85, 0.5), 3.0, st, egui::StrokeKind::Inside);
            line(pt(-0.5, -0.15), pt(0.5, -0.15));
            line(pt(-0.5, 0.15), pt(0.2, 0.15));
            tri(p, [pt(-0.4, 0.5), pt(-0.1, 0.5), pt(-0.5, 0.85)], c);
        }
        Icon::Rotate => {
            let pts: Vec<Pos2> = (0..=18).map(|i| {
                let a = -1.2 + i as f32 * 0.3;
                pt(0.65 * a.cos(), 0.65 * a.sin())
            }).collect();
            p.add(Shape::line(pts.clone(), st));
            let e = *pts.last().unwrap();
            tri(p, [e + vec2(-0.3 * s, -0.3 * s), e + vec2(0.35 * s, -0.1 * s), e + vec2(0.0, 0.35 * s)], c);
        }
        Icon::Fit => {
            p.rect_stroke(fr(-0.8, -0.6, 0.8, 0.6), 2.0, st, egui::StrokeKind::Inside);
            p.rect_stroke(fr(-0.4, -0.25, 0.4, 0.25), 0.0, st, egui::StrokeKind::Inside);
        }
        Icon::ZoomIn | Icon::ZoomOut => {
            p.circle_stroke(pt(-0.15, -0.15), s * 0.55, st);
            line(pt(0.25, 0.25), pt(0.8, 0.8));
            line(pt(-0.4, -0.15), pt(0.1, -0.15));
            if ic == Icon::ZoomIn {
                line(pt(-0.15, -0.4), pt(-0.15, 0.1));
            }
        }
        Icon::Copy => {
            p.rect_stroke(fr(-0.4, -0.8, 0.8, 0.4), 2.0, st, egui::StrokeKind::Inside);
            p.rect_stroke(fr(-0.8, -0.4, 0.4, 0.8), 2.0, st, egui::StrokeKind::Inside);
        }
        Icon::Slideshow => {
            p.rect_stroke(fr(-0.85, -0.65, 0.85, 0.65), 2.0, st, egui::StrokeKind::Inside);
            tri(p, [pt(-0.2, -0.3), pt(-0.2, 0.3), pt(0.3, 0.0)], c);
        }
        Icon::Plus => {
            line(pt(-0.6, 0.0), pt(0.6, 0.0));
            line(pt(0.0, -0.6), pt(0.0, 0.6));
        }
        Icon::Close => {
            line(pt(-0.55, -0.55), pt(0.55, 0.55));
            line(pt(-0.55, 0.55), pt(0.55, -0.55));
        }
        Icon::Note => {
            p.circle_filled(pt(-0.35, 0.55), s * 0.32, c);
            line(pt(-0.05, 0.5), pt(-0.05, -0.8));
            line(pt(-0.05, -0.8), pt(0.6, -0.5));
        }
        Icon::Film => {
            p.rect_stroke(fr(-0.8, -0.6, 0.8, 0.6), 2.0, st, egui::StrokeKind::Inside);
            tri(p, [pt(-0.2, -0.3), pt(-0.2, 0.3), pt(0.3, 0.0)], c);
        }
        Icon::Folder => {
            p.add(Shape::closed_line(vec![pt(-0.85, -0.5), pt(-0.3, -0.5), pt(-0.1, -0.3), pt(0.85, -0.3), pt(0.85, 0.6), pt(-0.85, 0.6)], st));
        }
        Icon::Gear => {
            p.circle_stroke(pt(0.0, 0.0), s * 0.38, st);
            for k in 0..8 {
                let a = k as f32 * std::f32::consts::FRAC_PI_4;
                line(pt(0.55 * a.cos(), 0.55 * a.sin()), pt(0.85 * a.cos(), 0.85 * a.sin()));
            }
        }
        Icon::Tools => {
            line(pt(-0.7, 0.7), pt(0.3, -0.3));
            p.circle_stroke(pt(0.45, -0.45), s * 0.38, st);
            line(pt(0.7, 0.7), pt(-0.1, -0.1));
        }
        Icon::Stop => {
            p.rect_filled(fr(-0.6, -0.6, 0.6, 0.6), 2.0, c);
        }
        Icon::Trash => {
            line(pt(-0.75, -0.5), pt(0.75, -0.5));
            line(pt(-0.25, -0.5), pt(-0.25, -0.75));
            line(pt(0.25, -0.5), pt(0.25, -0.75));
            line(pt(-0.25, -0.75), pt(0.25, -0.75));
            p.add(Shape::line(vec![pt(-0.55, -0.5), pt(-0.45, 0.8), pt(0.45, 0.8), pt(0.55, -0.5)], st));
        }
        Icon::Scissors => {
            line(pt(-0.5, -0.1), pt(0.8, -0.75));
            line(pt(-0.5, 0.1), pt(0.8, 0.75));
            p.circle_stroke(pt(-0.55, -0.45), s * 0.28, st);
            p.circle_stroke(pt(-0.55, 0.45), s * 0.28, st);
        }
        Icon::Sun => {
            p.circle_filled(pt(0.0, 0.0), s * 0.38, c);
            for k in 0..8 {
                let a = k as f32 * std::f32::consts::FRAC_PI_4;
                line(pt(0.6 * a.cos(), 0.6 * a.sin()), pt(0.9 * a.cos(), 0.9 * a.sin()));
            }
        }
        Icon::Moon => {
            let pts: Vec<Pos2> = (0..=20).map(|i| {
                let a = 0.7 + i as f32 * (2.0 * std::f32::consts::PI - 1.4) / 20.0;
                pt(0.62 * a.cos() - 0.05, 0.62 * a.sin())
            }).collect();
            p.add(Shape::line(pts, Stroke::new((s / 3.2).max(2.0), c)));
        }
        Icon::Doc => {
            p.add(Shape::closed_line(vec![pt(-0.6, -0.85), pt(0.2, -0.85), pt(0.6, -0.45), pt(0.6, 0.85), pt(-0.6, 0.85)], st));
            line(pt(-0.3, -0.1), pt(0.3, -0.1));
            line(pt(-0.3, 0.25), pt(0.3, 0.25));
            line(pt(-0.3, 0.6), pt(0.1, 0.6));
        }
        Icon::Clock => {
            p.circle_stroke(pt(0.0, 0.0), s * 0.85, st);
            line(pt(0.0, 0.0), pt(0.0, -0.5));
            line(pt(0.0, 0.0), pt(0.38, 0.22));
        }
        Icon::Sliders => {
            for (y, x) in [(-0.55f32, -0.3f32), (0.0, 0.35), (0.55, -0.1)] {
                line(pt(-0.8, y), pt(0.8, y));
                p.circle_filled(pt(x, y), s * 0.17, c);
            }
        }
        Icon::Check => {
            line(pt(-0.7, 0.05), pt(-0.2, 0.55));
            line(pt(-0.2, 0.55), pt(0.75, -0.55));
        }
        Icon::Pencil => {
            line(pt(-0.7, 0.7), pt(-0.55, 0.25));
            line(pt(-0.55, 0.25), pt(0.45, -0.75));
            line(pt(0.45, -0.75), pt(0.8, -0.4));
            line(pt(0.8, -0.4), pt(-0.2, 0.6));
            line(pt(-0.2, 0.6), pt(-0.7, 0.7));
        }
        Icon::Palette => {
            p.circle_stroke(pt(0.0, 0.0), s * 0.8, st);
            for (x, y) in [(-0.35f32, -0.25f32), (0.05, -0.45), (0.45, -0.1)] {
                p.circle_filled(pt(x, y), s * 0.13, c);
            }
        }
        Icon::Grid => {
            for (x, y) in [(-0.85f32, -0.85f32), (0.1, -0.85), (-0.85, 0.1), (0.1, 0.1)] {
                p.rect_filled(fr(x, y, x + 0.75, y + 0.75), 2.0, c);
            }
        }
        Icon::Merge => {
            line(pt(-0.8, -0.55), pt(0.0, 0.0));
            line(pt(-0.8, 0.55), pt(0.0, 0.0));
            line(pt(0.0, 0.0), pt(0.7, 0.0));
            tri(p, [pt(0.5, -0.3), pt(0.5, 0.3), pt(0.9, 0.0)], c);
        }
        Icon::Picture => {
            p.rect_stroke(fr(-0.8, -0.6, 0.8, 0.6), 2.0, st, egui::StrokeKind::Inside);
            tri(p, [pt(-0.6, 0.45), pt(-0.15, -0.2), pt(0.3, 0.45)], c);
            p.circle_filled(pt(0.4, -0.25), s * 0.15, c);
        }
    }
}

/// Bouton-icône (survol, état « actif », info-bulle).
pub fn button(ui: &mut Ui, ic: Icon, size: f32, active: bool, tip: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(size + 12.0, size + 8.0), Sense::click());
    let accent = ui.visuals().selection.bg_fill;
    if resp.hovered() || active {
        let f = if active { accent.gamma_multiply(0.3) } else { ui.visuals().widgets.hovered.weak_bg_fill };
        ui.painter().rect_filled(rect, 5.0, f);
    }
    let c = if active { accent } else if resp.hovered() { ui.visuals().strong_text_color() } else { ui.visuals().text_color() };
    paint(ui.painter(), Rect::from_center_size(rect.center(), vec2(size, size)), ic, c);
    if tip.is_empty() { resp } else { resp.on_hover_text(tip) }
}

/// Bouton « icône + texte » ; `fill` colore le bouton (action principale), sinon style standard.
pub fn labeled(ui: &mut Ui, ic: Icon, text: &str, fill: Option<Color32>, enabled: bool, tip: &str) -> Response {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let fg = if fill.is_some() { Color32::WHITE } else { ui.visuals().text_color() };
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, fg);
    let isz = 16.0;
    let pad = ui.spacing().button_padding;
    let w = pad.x * 2.0 + if text.is_empty() { isz } else { isz + 6.0 + galley.size().x };
    let h = (galley.size().y.max(isz) + pad.y * 2.0).max(ui.spacing().interact_size.y);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, h), if enabled { Sense::click() } else { Sense::hover() });
    let v = ui.visuals();
    let (bg, stroke) = match (fill, resp.hovered() && enabled) {
        (Some(f), true) => (f.gamma_multiply(1.12), Stroke::NONE),
        (Some(f), false) => (f, Stroke::NONE),
        (None, true) => (v.widgets.hovered.weak_bg_fill, v.widgets.hovered.bg_stroke),
        (None, false) => (v.widgets.inactive.weak_bg_fill, v.widgets.inactive.bg_stroke),
    };
    let alpha = if enabled { 1.0 } else { 0.4 };
    ui.painter().rect(rect, 6.0, bg.gamma_multiply(if fill.is_some() { alpha } else { 1.0 }), stroke, egui::StrokeKind::Inside);
    let c = fg.gamma_multiply(alpha);
    let icon_rect = Rect::from_center_size(pos2(rect.left() + pad.x + isz / 2.0, rect.center().y), vec2(isz, isz));
    paint(ui.painter(), icon_rect, ic, c);
    if !text.is_empty() {
        ui.painter().galley(pos2(icon_rect.right() + 6.0, rect.center().y - galley.size().y / 2.0), galley, c);
    }
    if tip.is_empty() { resp } else { resp.on_hover_text(tip) }
}
