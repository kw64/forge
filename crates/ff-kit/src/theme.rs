//! Thèmes : trois styles (coloré, sobre, office), chacun en clair et en sombre. v0.5.0
use egui::{epaint::CornerRadius, Color32, Context, Stroke, Visuals};
use std::sync::RwLock;

static CURRENT: RwLock<Option<Palette>> = RwLock::new(None);

/// Palette du thème appliqué en dernier (Coloré sombre avant tout appel à `apply`).
pub fn current() -> Palette {
    CURRENT.read().ok().and_then(|g| *g).unwrap_or_else(|| palette(Style::Colorful, true))
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Style {
    Colorful,
    Sober,
    Office,
    /// Rétro Workbench (gris/bleu/orange), clin d'œil à l'Amiga.
    Amiga,
}

pub const STYLES: [Style; 4] = [Style::Colorful, Style::Sober, Style::Office, Style::Amiga];

#[derive(Clone, Copy)]
pub struct Palette {
    pub accent: Color32,
    pub ok: Color32,
    pub err: Color32,
    pub bg: Color32,
    pub panel: Color32,
    pub card: Color32,
    pub text: Color32,
    pub weak: Color32,
    pub stroke: Color32,
    /// Bandeau d'en-tête (couleur pleine pour « coloré » et « office »).
    pub header: Option<Color32>,
}

impl Style {
    pub fn id(self) -> &'static str {
        match self {
            Style::Colorful => "colorful",
            Style::Sober => "sober",
            Style::Office => "office",
            Style::Amiga => "amiga",
        }
    }
    pub fn from_id(s: &str) -> Style {
        STYLES.iter().copied().find(|x| x.id() == s).unwrap_or(Style::Colorful)
    }
    pub fn label(self, en: bool) -> &'static str {
        match (self, en) {
            (Style::Colorful, false) => "Coloré",
            (Style::Colorful, true) => "Colorful",
            (Style::Sober, false) => "Sobre",
            (Style::Sober, true) => "Sober",
            (Style::Office, _) => "Office",
            (Style::Amiga, _) => "Amiga",
        }
    }
}

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

pub fn palette(style: Style, dark: bool) -> Palette {
    let ok = if dark { rgb(0x4A, 0xDE, 0x80) } else { rgb(0x16, 0xA3, 0x4A) };
    let err = if dark { rgb(0xF8, 0x71, 0x71) } else { rgb(0xDC, 0x26, 0x26) };
    match (style, dark) {
        (Style::Colorful, false) => Palette { accent: rgb(0xE8, 0x59, 0x0C), ok, err, bg: rgb(0xFF, 0xF7, 0xF0), panel: rgb(0xFF, 0xEC, 0xDC), card: rgb(0xFF, 0xFF, 0xFF), text: rgb(0x2B, 0x1B, 0x12), weak: rgb(0x8A, 0x6F, 0x5E), stroke: rgb(0xF2, 0xC9, 0xA8), header: Some(rgb(0xE8, 0x59, 0x0C)) },
        (Style::Colorful, true) => Palette { accent: rgb(0xFF, 0x7A, 0x2F), ok, err, bg: rgb(0x16, 0x13, 0x1F), panel: rgb(0x22, 0x1A, 0x33), card: rgb(0x2B, 0x22, 0x40), text: rgb(0xF3, 0xEE, 0xFF), weak: rgb(0xA9, 0x9B, 0xC7), stroke: rgb(0x45, 0x37, 0x66), header: Some(rgb(0x6D, 0x28, 0xD9)) },
        (Style::Sober, false) => Palette { accent: rgb(0x47, 0x55, 0x69), ok, err, bg: rgb(0xF5, 0xF5, 0xF4), panel: rgb(0xEC, 0xEC, 0xEA), card: rgb(0xFF, 0xFF, 0xFF), text: rgb(0x1F, 0x23, 0x28), weak: rgb(0x6B, 0x72, 0x80), stroke: rgb(0xD4, 0xD4, 0xD0), header: None },
        (Style::Sober, true) => Palette { accent: rgb(0x9C, 0xA9, 0xBC), ok, err, bg: rgb(0x1A, 0x1B, 0x1D), panel: rgb(0x22, 0x23, 0x26), card: rgb(0x2A, 0x2B, 0x2F), text: rgb(0xE5, 0xE7, 0xEB), weak: rgb(0x8B, 0x91, 0x9C), stroke: rgb(0x3A, 0x3C, 0x41), header: None },
        (Style::Office, false) => Palette { accent: rgb(0x2B, 0x57, 0x9A), ok, err, bg: rgb(0xFF, 0xFF, 0xFF), panel: rgb(0xF3, 0xF3, 0xF3), card: rgb(0xFF, 0xFF, 0xFF), text: rgb(0x26, 0x26, 0x26), weak: rgb(0x6E, 0x6E, 0x6E), stroke: rgb(0xD1, 0xD1, 0xD1), header: Some(rgb(0x2B, 0x57, 0x9A)) },
        (Style::Amiga, false) => Palette { accent: rgb(0x00, 0x55, 0xAA), ok: rgb(0x00, 0x7A, 0x2E), err: rgb(0xCC, 0x22, 0x00), bg: rgb(0xAA, 0xAA, 0xAA), panel: rgb(0x99, 0x99, 0x99), card: rgb(0xAA, 0xAA, 0xAA), text: rgb(0x00, 0x00, 0x00), weak: rgb(0x44, 0x44, 0x44), stroke: rgb(0x55, 0x55, 0x55), header: Some(rgb(0x00, 0x55, 0xAA)) },
        (Style::Amiga, true) => Palette { accent: rgb(0xFF, 0x88, 0x00), ok: rgb(0x55, 0xFF, 0x55), err: rgb(0xFF, 0x55, 0x55), bg: rgb(0x00, 0x10, 0x30), panel: rgb(0x00, 0x22, 0x55), card: rgb(0x00, 0x18, 0x40), text: rgb(0xFF, 0xFF, 0xFF), weak: rgb(0x88, 0xAA, 0xDD), stroke: rgb(0x00, 0x55, 0xAA), header: Some(rgb(0x00, 0x55, 0xAA)) },
        (Style::Office, true) => Palette { accent: rgb(0x4F, 0x8F, 0xE0), ok, err, bg: rgb(0x2B, 0x2B, 0x2B), panel: rgb(0x33, 0x33, 0x33), card: rgb(0x3C, 0x3C, 0x3C), text: rgb(0xF0, 0xF0, 0xF0), weak: rgb(0xA0, 0xA0, 0xA0), stroke: rgb(0x55, 0x55, 0x55), header: Some(rgb(0x1F, 0x3A, 0x66)) },
    }
}

/// Texte lisible (noir ou blanc) sur une couleur de fond.
pub fn on_color(c: Color32) -> Color32 {
    let l = 0.299 * c.r() as f32 + 0.587 * c.g() as f32 + 0.114 * c.b() as f32;
    if l > 160.0 { Color32::BLACK } else { Color32::WHITE }
}

/// Applique le thème à egui (couleurs, arrondis, contrastes).
pub fn apply(ctx: &Context, style: Style, dark: bool) {
    let p = palette(style, dark);
    if let Ok(mut g) = CURRENT.write() {
        *g = Some(p);
    }
    let mut v = if dark { Visuals::dark() } else { Visuals::light() };
    v.panel_fill = p.panel;
    v.window_fill = p.card;
    v.extreme_bg_color = p.card;
    v.faint_bg_color = p.bg;
    v.override_text_color = Some(p.text);
    v.hyperlink_color = p.accent;
    v.selection.bg_fill = p.accent;
    v.selection.stroke = Stroke::new(1.0, on_color(p.accent));
    v.window_stroke = Stroke::new(1.0, p.stroke);
    let r = match style {
        Style::Colorful => 10,
        Style::Sober => 4,
        Style::Office => 2,
        Style::Amiga => 0,
    };
    let cr = CornerRadius::same(r);
    v.window_corner_radius = CornerRadius::same(r + 2);
    for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.corner_radius = cr;
    }
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.stroke);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    v.widgets.inactive.weak_bg_fill = p.card;
    v.widgets.inactive.bg_fill = p.stroke;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, p.stroke);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, p.text);
    v.widgets.hovered.weak_bg_fill = p.accent.gamma_multiply(0.18);
    v.widgets.hovered.bg_fill = p.accent.gamma_multiply(0.25);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, p.accent);
    v.widgets.hovered.fg_stroke = Stroke::new(1.5, p.text);
    v.widgets.active.weak_bg_fill = p.accent.gamma_multiply(0.35);
    v.widgets.active.bg_fill = p.accent;
    v.widgets.active.bg_stroke = Stroke::new(1.0, p.accent);
    v.widgets.active.fg_stroke = Stroke::new(1.5, p.text);
    ctx.set_visuals(v);
    ctx.all_styles_mut(|s| {
        s.spacing.button_padding = match style {
            Style::Office => egui::vec2(8.0, 4.0),
            _ => egui::vec2(10.0, 5.0),
        };
    });
}

/// Sélecteur de style (pastilles) ; retourne vrai si l'utilisateur a changé `current_id`.
pub fn style_picker(ui: &mut egui::Ui, current_id: &mut String, dark: bool, en: bool) -> bool {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        for st in STYLES {
            let p = palette(st, dark);
            let sel = current_id.as_str() == st.id();
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(132.0, 68.0), egui::Sense::click());
            let painter = ui.painter();
            painter.rect(rect, 8.0, p.bg, Stroke::new(if sel { 2.5 } else { 1.0 }, if sel { p.accent } else { p.stroke }), egui::StrokeKind::Inside);
            painter.rect_filled(egui::Rect::from_min_size(rect.min + egui::vec2(6.0, 6.0), egui::vec2(rect.width() - 12.0, 12.0)), 4.0, p.header.unwrap_or(p.panel));
            for (i, c) in [p.accent, p.ok, p.err].into_iter().enumerate() {
                painter.circle_filled(rect.left_center() + egui::vec2(16.0 + 18.0 * i as f32, -2.0), 6.0, c);
            }
            painter.text(rect.center_bottom() + egui::vec2(0.0, -7.0), egui::Align2::CENTER_BOTTOM, st.label(en), egui::FontId::proportional(13.0), p.text);
            if resp.clicked() && !sel {
                *current_id = st.id().to_string();
                changed = true;
            }
        }
    });
    changed
}
