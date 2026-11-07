//! Éditeur visuel de profil vidéo : codec, qualité, taille, audio, filtres et incrustations (timecode, texte, logo PNG). v0.15.0
use crate::i18n::tr;
use eframe::egui::{self, Color32, RichText};
use ff_core::spec::{LogoOverlay, Pos, TextOverlay, VideoSpec};
use ff_core::Lang;
use std::collections::HashSet;

/// Codecs vidéo : (id, libellé, encodeur FFmpeg requis).
const VCODECS: &[(&str, &str, &str)] = &[
    ("copy", "Copie (sans réencodage)", ""),
    ("h264", "H.264 (x264)", "libx264"),
    ("h265", "H.265 / HEVC (x265)", "libx265"),
    ("vp9", "VP9", "libvpx-vp9"),
    ("av1", "AV1 (SVT-AV1)", "libsvtav1"),
    ("prores", "Apple ProRes", "prores_ks"),
];

const ACODECS: &[(&str, &str, &str)] = &[
    ("aac", "AAC", "aac"),
    ("mp3", "MP3", "libmp3lame"),
    ("opus", "Opus", "libopus"),
    ("ac3", "AC-3 (Dolby Digital)", "ac3"),
    ("flac", "FLAC (sans perte)", "flac"),
    ("copy", "Copie de l'audio", ""),
    ("none", "Sans audio", ""),
];

const PRESETS: &[&str] = &["ultrafast", "superfast", "veryfast", "faster", "fast", "medium", "slow", "slower", "veryslow"];

/// Modèles de départ : (libellé, spec).
pub fn templates() -> Vec<(&'static str, VideoSpec)> {
    let d = VideoSpec::default;
    vec![
        ("MP4 universel (H.264)", d()),
        ("MP4 léger 720p", VideoSpec { max_h: 720, crf: 28, abitrate: 96, ..d() }),
        ("MKV HDLight 1080p (x265 10 bits)", VideoSpec { container: "mkv".into(), vcodec: "h265".into(), crf: 23, tenbit: true, max_h: 1080, all_tracks: true, abitrate: 256, faststart: false, ..d() }),
        ("MKV HDLight 720p (x264)", VideoSpec { container: "mkv".into(), crf: 21, preset: "slow".into(), max_h: 720, all_tracks: true, abitrate: 192, faststart: false, ..d() }),
        ("WebM (VP9 + Opus)", VideoSpec { container: "webm".into(), vcodec: "vp9".into(), crf: 32, acodec: "opus".into(), abitrate: 96, faststart: false, ..d() }),
        ("Derushage : timecode incrusté 720p", VideoSpec { max_h: 720, crf: 26, timecode: ff_core::spec::TimecodeOverlay { enabled: true, ..Default::default() }, ..d() }),
    ]
}

fn hex_to_color(h: &str) -> Color32 {
    let h = h.trim().trim_start_matches('#');
    u32::from_str_radix(h, 16).ok().filter(|_| h.len() == 6).map(|v| Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)).unwrap_or(Color32::WHITE)
}

fn color_edit(ui: &mut egui::Ui, hex: &mut String) {
    let mut c = hex_to_color(hex);
    let mut rgb = [c.r(), c.g(), c.b()];
    if ui.color_edit_button_srgb(&mut rgb).changed() {
        c = Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
        *hex = format!("#{:02X}{:02X}{:02X}", c.r(), c.g(), c.b());
    }
}

/// Grille 3×3 de positions.
fn pos_grid(ui: &mut egui::Ui, id: &str, pos: &mut Pos) {
    egui::Grid::new(id).spacing([2.0, 2.0]).show(ui, |ui| {
        for row in 0..3u8 {
            for col in 0..3u8 {
                let p = row * 3 + col;
                let glyph = ["↖", "↑", "↗", "←", "•", "→", "↙", "↓", "↘"][p as usize];
                if ui.add_sized([24.0, 22.0], egui::Button::selectable(*pos == p, glyph)).clicked() {
                    *pos = p;
                }
            }
            ui.end_row();
        }
    });
}

fn combo_u32(ui: &mut egui::Ui, id: &str, l: Lang, val: &mut u32, opts: &[(u32, &str)]) {
    let cur = opts.iter().find(|(v, _)| v == val).map(|(_, n)| tr(l, n).to_string()).unwrap_or_else(|| val.to_string());
    egui::ComboBox::from_id_salt(id).selected_text(cur).show_ui(ui, |ui| {
        for (v, n) in opts {
            if ui.selectable_label(val == v, tr(l, n)).clicked() {
                *val = *v;
            }
        }
    });
}

/// Réglages communs d'une incrustation de texte : taille, couleur, fond, marge.
fn style_row(ui: &mut egui::Ui, l: Lang, size: &mut f32, color: &mut String, boxed: &mut bool, opacity: &mut u8, margin: &mut u32) {
    ui.horizontal_wrapped(|ui| {
        ui.label(tr(l, "Taille"));
        ui.add(egui::DragValue::new(size).range(8.0..=200.0).speed(0.5));
        ui.label(tr(l, "Couleur"));
        color_edit(ui, color);
        ui.checkbox(boxed, tr(l, "Fond"));
        if *boxed {
            ui.add(egui::DragValue::new(opacity).range(0..=100).suffix(" %"));
        }
        ui.label(tr(l, "Marge"));
        ui.add(egui::DragValue::new(margin).range(0..=400));
    });
}

fn section(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    egui::CollapsingHeader::new(RichText::new(title).strong()).default_open(true).show(ui, add);
}

/// Édite `s` ; renvoie `true` si le modèle a changé. `enc` : encodeurs disponibles (vide = inconnu, tout est permis).
pub fn editor(ui: &mut egui::Ui, l: Lang, s: &mut VideoSpec, enc: &HashSet<String>) -> bool {
    let before = s.clone();
    let have = |e: &str| e.is_empty() || enc.is_empty() || enc.contains(e);

    ui.horizontal(|ui| {
        ui.label(tr(l, "Partir d'un modèle :"));
        egui::ComboBox::from_id_salt("spec_tpl").selected_text(tr(l, "Choisir…")).show_ui(ui, |ui| {
            for (n, t) in templates() {
                if ui.selectable_label(false, tr(l, n)).clicked() {
                    let (font, logo) = (s.font.clone(), s.logo.clone());
                    *s = t;
                    s.font = font;
                    s.logo = logo;
                }
            }
        });
    });

    section(ui, tr(l, "Format et codec vidéo"), |ui| {
        egui::Grid::new("spec_fmt").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
            ui.label(tr(l, "Conteneur"));
            egui::ComboBox::from_id_salt("spec_cont").selected_text(format!(".{}", s.container)).show_ui(ui, |ui| {
                for (c, n) in [("mp4", "MP4 (compatible partout)"), ("mkv", "MKV (pistes multiples, HDLight)"), ("webm", "WebM (web)"), ("mov", "MOV (montage, ProRes)")] {
                    if ui.selectable_label(s.container == c, tr(l, n)).clicked() {
                        s.container = c.into();
                    }
                }
            });
            ui.end_row();
            ui.label(tr(l, "Codec vidéo"));
            let cur = VCODECS.iter().find(|(i, _, _)| *i == s.vcodec).map(|(_, n, _)| tr(l, n)).unwrap_or("?");
            egui::ComboBox::from_id_salt("spec_vc").selected_text(cur).show_ui(ui, |ui| {
                for (i, n, e) in VCODECS {
                    let ok = have(e);
                    let txt = if ok { tr(l, n).to_string() } else { format!("{} — {}", tr(l, n), tr(l, "indisponible")) };
                    if ui.add_enabled(ok, egui::Button::selectable(s.vcodec == *i, txt)).clicked() {
                        s.vcodec = (*i).into();
                    }
                }
            });
            ui.end_row();
            if s.vcodec != "copy" && s.vcodec != "prores" {
                ui.label(tr(l, "Qualité"));
                ui.horizontal(|ui| {
                    ui.radio_value(&mut s.bitrate_mode, false, tr(l, "Constante (CRF)"));
                    ui.radio_value(&mut s.bitrate_mode, true, tr(l, "Débit cible"));
                });
                ui.end_row();
                if s.bitrate_mode {
                    ui.label(tr(l, "Débit vidéo"));
                    ui.add(egui::DragValue::new(&mut s.vbitrate).range(100..=100_000).speed(20.0).suffix(" kb/s"));
                } else {
                    ui.label(tr(l, "CRF (bas = meilleur)"));
                    ui.add(egui::Slider::new(&mut s.crf, 10..=40));
                }
                ui.end_row();
                if matches!(s.vcodec.as_str(), "h264" | "h265") {
                    ui.label(tr(l, "Vitesse (preset)"));
                    egui::ComboBox::from_id_salt("spec_preset").selected_text(s.preset.clone()).show_ui(ui, |ui| {
                        for p in PRESETS {
                            if ui.selectable_label(s.preset == *p, *p).clicked() {
                                s.preset = (*p).into();
                            }
                        }
                    });
                    ui.end_row();
                    ui.label(tr(l, "Réglage (tune)"));
                    egui::ComboBox::from_id_salt("spec_tune").selected_text(if s.tune.is_empty() { tr(l, "Aucun") } else { &s.tune }).show_ui(ui, |ui| {
                        for (t, n) in [("", "Aucun"), ("film", "film"), ("animation", "animation"), ("grain", "grain"), ("fastdecode", "fastdecode")] {
                            if ui.selectable_label(s.tune == t, tr(l, n)).clicked() {
                                s.tune = t.into();
                            }
                        }
                    });
                    ui.end_row();
                }
                ui.label(tr(l, "Profondeur"));
                ui.checkbox(&mut s.tenbit, tr(l, "10 bits (meilleur dégradé, fichiers plus compacts)"));
                ui.end_row();
            }
        });
    });

    section(ui, tr(l, "Taille et image"), |ui| {
        egui::Grid::new("spec_size").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
            ui.label(tr(l, "Hauteur maximale"));
            combo_u32(ui, "spec_h", l, &mut s.max_h, &[(0, "Inchangée"), (2160, "2160 (4K)"), (1440, "1440"), (1080, "1080 (Full HD)"), (720, "720 (HD)"), (576, "576"), (480, "480")]);
            ui.end_row();
            ui.label(tr(l, "Largeur maximale"));
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut s.max_w).range(0..=7680).speed(8.0));
                ui.label(RichText::new(tr(l, "0 = inchangée ; jamais d'agrandissement")).small().weak());
            });
            ui.end_row();
            ui.label(tr(l, "Images par seconde"));
            let mut f = s.fps as u32;
            combo_u32(ui, "spec_fps", l, &mut f, &[(0, "Inchangées"), (24, "24"), (25, "25"), (30, "30"), (50, "50"), (60, "60")]);
            s.fps = f as f32;
            ui.end_row();
            ui.label(tr(l, "Filtres"));
            ui.horizontal(|ui| {
                ui.checkbox(&mut s.deinterlace, tr(l, "Désentrelacer"));
                let mut d = s.denoise as u32;
                ui.label(tr(l, "Débruitage"));
                combo_u32(ui, "spec_dn", l, &mut d, &[(0, "Aucun"), (1, "Léger"), (2, "Moyen"), (3, "Fort")]);
                s.denoise = d as u8;
            });
            ui.end_row();
        });
    });

    section(ui, tr(l, "Audio et pistes"), |ui| {
        egui::Grid::new("spec_audio").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
            ui.label(tr(l, "Codec audio"));
            let cur = ACODECS.iter().find(|(i, _, _)| *i == s.acodec).map(|(_, n, _)| tr(l, n)).unwrap_or("?");
            egui::ComboBox::from_id_salt("spec_ac").selected_text(cur).show_ui(ui, |ui| {
                for (i, n, e) in ACODECS {
                    let ok = have(e);
                    let txt = if ok { tr(l, n).to_string() } else { format!("{} — {}", tr(l, n), tr(l, "indisponible")) };
                    if ui.add_enabled(ok, egui::Button::selectable(s.acodec == *i, txt)).clicked() {
                        s.acodec = (*i).into();
                    }
                }
            });
            ui.end_row();
            if !matches!(s.acodec.as_str(), "copy" | "none" | "flac") {
                ui.label(tr(l, "Débit audio"));
                ui.add(egui::DragValue::new(&mut s.abitrate).range(32..=640).speed(1.0).suffix(" kb/s"));
                ui.end_row();
            }
            if !matches!(s.acodec.as_str(), "copy" | "none") {
                ui.label(tr(l, "Canaux"));
                let mut c = s.channels as u32;
                combo_u32(ui, "spec_ch", l, &mut c, &[(0, "Inchangés"), (1, "Mono"), (2, "Stéréo"), (6, "5.1")]);
                s.channels = c as u8;
                ui.end_row();
            }
            ui.label(tr(l, "Pistes"));
            ui.checkbox(&mut s.all_tracks, tr(l, "Conserver toutes les pistes audio et les sous-titres"));
            ui.end_row();
            if matches!(s.container.as_str(), "mp4" | "mov") {
                ui.label("");
                ui.checkbox(&mut s.faststart, tr(l, "Démarrage rapide (faststart, lecture web)"));
                ui.end_row();
            }
        });
    });

    section(ui, tr(l, "Incrustations sur l'image"), |ui| {
        // Timecode
        ui.checkbox(&mut s.timecode.enabled, RichText::new(tr(l, "Timecode incrusté")).strong());
        if s.timecode.enabled {
            let t = &mut s.timecode;
            ui.indent("tc", |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(tr(l, "Format"));
                    for (m, n) in [(0u8, "SMPTE HH:MM:SS:FF"), (1, "Temps HH:MM:SS.mmm"), (2, "N° d'image")] {
                        ui.radio_value(&mut t.mode, m, tr(l, n));
                    }
                });
                if t.mode == 0 {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(tr(l, "Départ"));
                        ui.add(egui::TextEdit::singleline(&mut t.start).desired_width(90.0).font(egui::TextStyle::Monospace));
                        ui.label(tr(l, "Cadence"));
                        ui.add(egui::TextEdit::singleline(&mut t.rate).desired_width(70.0));
                        ui.checkbox(&mut t.drop_frame, tr(l, "Drop-frame (29,97 / 59,94)"));
                    });
                    ui.label(RichText::new(tr(l, "Cadence « auto » : lue dans chaque fichier. Le timecode part de la valeur de départ au début de la sortie.")).small().weak());
                }
                ui.horizontal(|ui| {
                    ui.label(tr(l, "Position"));
                    pos_grid(ui, "tc_pos", &mut t.pos);
                });
                style_row(ui, l, &mut t.size, &mut t.color, &mut t.boxed, &mut t.box_opacity, &mut t.margin);
            });
        }
        ui.separator();
        // Textes
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr(l, "Textes")).strong());
            if ui.button(tr(l, "+ Ajouter un texte")).clicked() {
                s.texts.push(TextOverlay::default());
            }
        });
        let mut remove = None;
        for (i, t) in s.texts.iter_mut().enumerate() {
            ui.push_id(("txt", i), |ui| {
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut t.text).desired_width(260.0).hint_text(tr(l, "Texte à afficher")));
                        if ui.button("✕").on_hover_text(tr(l, "Retirer")).clicked() {
                            remove = Some(i);
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label(tr(l, "Position"));
                        pos_grid(ui, "tx_pos", &mut t.pos);
                        ui.vertical(|ui| {
                            style_row(ui, l, &mut t.size, &mut t.color, &mut t.boxed, &mut t.box_opacity, &mut t.margin);
                            ui.horizontal(|ui| {
                                ui.label(tr(l, "Visible de (s)"));
                                ui.add(egui::DragValue::new(&mut t.start).range(0.0..=86400.0).speed(0.1));
                                ui.label(tr(l, "à (s)"));
                                ui.add(egui::DragValue::new(&mut t.end).range(0.0..=86400.0).speed(0.1));
                                ui.label(RichText::new(tr(l, "0 et 0 = toute la durée")).small().weak());
                            });
                        });
                    });
                });
            });
        }
        if let Some(i) = remove {
            s.texts.remove(i);
        }
        ui.separator();
        // Logo
        let mut use_logo = s.logo.is_some();
        if ui.checkbox(&mut use_logo, RichText::new(tr(l, "Logo (image PNG)")).strong()).changed() {
            s.logo = if use_logo { Some(LogoOverlay::default()) } else { None };
        }
        if let Some(lg) = &mut s.logo {
            ui.indent("logo", |ui| {
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut lg.path).desired_width(300.0).hint_text(tr(l, "Chemin du logo .png")));
                    if ui.button(tr(l, "Parcourir…")).clicked() {
                        if let Some(p) = rfd::FileDialog::new().add_filter("PNG / image", &["png", "jpg", "jpeg", "webp"]).pick_file() {
                            lg.path = p.display().to_string();
                        }
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(tr(l, "Position"));
                    pos_grid(ui, "lg_pos", &mut lg.pos);
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(tr(l, "Largeur (% de l'image)"));
                            ui.add(egui::Slider::new(&mut lg.width_pct, 2.0..=100.0));
                        });
                        ui.horizontal(|ui| {
                            ui.label(tr(l, "Opacité"));
                            ui.add(egui::Slider::new(&mut lg.opacity, 5..=100).suffix(" %"));
                            ui.label(tr(l, "Marge"));
                            ui.add(egui::DragValue::new(&mut lg.margin).range(0..=400));
                        });
                    });
                });
            });
        }
    });

    ui.collapsing(tr(l, "Options avancées"), |ui| {
        ui.horizontal(|ui| {
            ui.label(tr(l, "Police des textes (.ttf)"));
            ui.add(egui::TextEdit::singleline(&mut s.font).desired_width(300.0).hint_text(ff_core::spec::default_font()));
            if ui.button(tr(l, "Parcourir…")).clicked() {
                if let Some(p) = rfd::FileDialog::new().add_filter("TrueType / OpenType", &["ttf", "otf", "ttc"]).pick_file() {
                    s.font = p.display().to_string();
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label(tr(l, "Arguments FFmpeg supplémentaires"));
            ui.add(egui::TextEdit::singleline(&mut s.extra).desired_width(300.0).font(egui::TextStyle::Monospace));
        });
    });

    for p in s.problems() {
        ui.colored_label(crate::look::red(), tr(l, p));
    }
    *s != before
}
