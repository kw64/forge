//! Profil vidéo « visuel » : un modèle structuré (codec, qualité, taille, audio, filtres, incrustations texte / timecode / logo)
//! converti en arguments FFmpeg. Aucune dépendance à FFmpeg : testable seul. v0.15.0
use serde::{Deserialize, Serialize};

/// Position sur une grille 3×3 : 0 haut-gauche, 1 haut-centre, 2 haut-droite, 3..5 milieu, 6..8 bas.
pub type Pos = u8;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TextOverlay {
    pub text: String,
    pub pos: Pos,
    /// Taille en « points » pour une image de 1080 px de haut (proportionnelle ensuite).
    pub size: f32,
    pub color: String,
    pub boxed: bool,
    /// Opacité du fond, en %.
    pub box_opacity: u8,
    pub margin: u32,
    /// Apparition (secondes) ; 0/0 = toute la durée.
    pub start: f32,
    pub end: f32,
}

impl Default for TextOverlay {
    fn default() -> Self {
        Self { text: "Texte".into(), pos: 8, size: 32.0, color: "#FFFFFF".into(), boxed: true, box_opacity: 50, margin: 24, start: 0.0, end: 0.0 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TimecodeOverlay {
    pub enabled: bool,
    /// 0 SMPTE (HH:MM:SS:FF), 1 temps écoulé (HH:MM:SS.mmm), 2 numéro d'image.
    pub mode: u8,
    pub drop_frame: bool,
    /// Timecode de départ (SMPTE).
    pub start: String,
    /// Cadence : « auto » (lue dans le fichier) ou une valeur (25, 29.97, 30000/1001…).
    pub rate: String,
    pub pos: Pos,
    pub size: f32,
    pub color: String,
    pub boxed: bool,
    pub box_opacity: u8,
    pub margin: u32,
}

impl Default for TimecodeOverlay {
    fn default() -> Self {
        Self { enabled: false, mode: 0, drop_frame: false, start: "00:00:00:00".into(), rate: "auto".into(), pos: 0, size: 30.0, color: "#FFFFFF".into(), boxed: true, box_opacity: 55, margin: 24 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LogoOverlay {
    pub path: String,
    pub pos: Pos,
    /// Largeur du logo en % de la largeur de l'image.
    pub width_pct: f32,
    pub opacity: u8,
    pub margin: u32,
}

impl Default for LogoOverlay {
    fn default() -> Self {
        Self { path: String::new(), pos: 2, width_pct: 15.0, opacity: 100, margin: 24 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VideoSpec {
    /// mp4, mkv, webm, mov.
    pub container: String,
    /// copy, h264, h265, vp9, av1, prores.
    pub vcodec: String,
    /// false : qualité constante (CRF) ; true : débit cible.
    pub bitrate_mode: bool,
    pub crf: u32,
    pub vbitrate: u32,
    pub preset: String,
    /// "", film, animation, grain…
    pub tune: String,
    pub tenbit: bool,
    /// Dimensions maximales (0 = inchangé) ; l'image n'est jamais agrandie.
    pub max_w: u32,
    pub max_h: u32,
    /// Images par seconde (0 = inchangé).
    pub fps: f32,
    pub deinterlace: bool,
    /// 0 aucun, 1 léger, 2 moyen, 3 fort.
    pub denoise: u8,
    /// copy, aac, mp3, opus, ac3, flac, none.
    pub acodec: String,
    pub abitrate: u32,
    /// 0 inchangé, sinon nombre de canaux.
    pub channels: u8,
    /// Conserver toutes les pistes audio et les sous-titres.
    pub all_tracks: bool,
    pub faststart: bool,
    pub timecode: TimecodeOverlay,
    pub texts: Vec<TextOverlay>,
    pub logo: Option<LogoOverlay>,
    /// Police des textes (fichier .ttf) ; vide = police du système.
    pub font: String,
    /// Arguments FFmpeg supplémentaires (avancé).
    pub extra: String,
}

impl Default for VideoSpec {
    fn default() -> Self {
        Self {
            container: "mp4".into(),
            vcodec: "h264".into(),
            bitrate_mode: false,
            crf: 23,
            vbitrate: 4000,
            preset: "medium".into(),
            tune: String::new(),
            tenbit: false,
            max_w: 0,
            max_h: 0,
            fps: 0.0,
            deinterlace: false,
            denoise: 0,
            acodec: "aac".into(),
            abitrate: 160,
            channels: 0,
            all_tracks: false,
            faststart: true,
            timecode: TimecodeOverlay::default(),
            texts: Vec::new(),
            logo: None,
            font: String::new(),
            extra: String::new(),
        }
    }
}

/// Graphe vidéo : entrées supplémentaires (logo) et filtres.
pub struct Graph {
    pub inputs: Vec<String>,
    /// Chaîne simple (`-vf`) quand il n'y a pas de logo.
    pub vf: String,
    /// Graphe complet (`-filter_complex`, sortie `[v]`) quand il y a un logo.
    pub complex: Option<String>,
}

/// Échappement à deux niveaux des valeurs de filtres (options puis graphe).
fn esc2(s: &str) -> String {
    let l1: String = s.chars().flat_map(|c| if matches!(c, '\\' | '\'' | ':') { vec!['\\', c] } else { vec![c] }).collect();
    l1.chars().flat_map(|c| if matches!(c, '\\' | '\'' | '[' | ']' | ',' | ';') { vec!['\\', c] } else { vec![c] }).collect()
}

/// Texte pour `drawtext` : `\` et `%` neutralisés (expansion), puis échappement à deux niveaux.
fn text_arg(s: &str) -> String {
    let t0: String = s.chars().flat_map(|c| if matches!(c, '\\' | '%') { vec!['\\', c] } else { vec![c] }).collect();
    esc2(&t0)
}

fn color_arg(hex: &str, alpha: Option<u8>) -> String {
    let h = hex.trim().trim_start_matches('#');
    let h = if h.len() == 6 && h.chars().all(|c| c.is_ascii_hexdigit()) { h.to_ascii_uppercase() } else { "FFFFFF".into() };
    match alpha {
        Some(a) => format!("0x{h}@{:.2}", a.min(100) as f32 / 100.0),
        None => format!("0x{h}"),
    }
}

/// Expressions x/y de `drawtext` (w, h, text_w, text_h) pour une case de la grille.
fn text_xy(pos: Pos, m: u32) -> (String, String) {
    let x = match pos % 3 { 0 => format!("{m}"), 1 => "(w-text_w)/2".into(), _ => format!("w-text_w-{m}") };
    let y = match pos / 3 { 0 => format!("{m}"), 1 => "(h-text_h)/2".into(), _ => format!("h-text_h-{m}") };
    (x, y)
}

fn overlay_xy(pos: Pos, m: u32) -> (String, String) {
    let x = match pos % 3 { 0 => format!("{m}"), 1 => "(W-w)/2".into(), _ => format!("W-w-{m}") };
    let y = match pos / 3 { 0 => format!("{m}"), 1 => "(H-h)/2".into(), _ => format!("H-h-{m}") };
    (x, y)
}

pub fn default_font() -> &'static str {
    if cfg!(windows) { "C:/Windows/Fonts/arial.ttf" } else { "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf" }
}

impl VideoSpec {
    pub fn extension(&self) -> &str {
        match self.container.as_str() {
            "mkv" | "webm" | "mov" => &self.container,
            _ => "mp4",
        }
    }

    pub fn has_overlays(&self) -> bool {
        self.timecode.enabled || !self.texts.is_empty() || self.logo.as_ref().map(|l| !l.path.trim().is_empty()).unwrap_or(false)
    }

    pub fn has_filters(&self) -> bool {
        self.has_overlays() || self.max_w > 0 || self.max_h > 0 || self.fps > 0.0 || self.deinterlace || self.denoise > 0
    }

    /// Problèmes empêchant la création du profil (message traduisible côté interface).
    pub fn problems(&self) -> Vec<&'static str> {
        let mut p = Vec::new();
        if self.vcodec == "copy" && self.has_filters() {
            p.push("La copie du flux vidéo est incompatible avec les filtres, la taille et les incrustations.");
        }
        if self.container == "webm" && !matches!(self.vcodec.as_str(), "vp9" | "av1") {
            p.push("WebM n'accepte que VP9 ou AV1 pour la vidéo.");
        }
        if self.container == "webm" && !matches!(self.acodec.as_str(), "opus" | "none") {
            p.push("WebM n'accepte que l'audio Opus.");
        }
        if self.vcodec == "prores" && self.container != "mov" && self.container != "mkv" {
            p.push("ProRes se range dans un conteneur MOV ou MKV.");
        }
        if let Some(l) = &self.logo {
            if l.path.trim().is_empty() {
                p.push("Choisissez le fichier PNG du logo (ou retirez-le).");
            }
        }
        p
    }

    fn font_arg(&self) -> String {
        let f = if self.font.trim().is_empty() { default_font() } else { self.font.trim() };
        esc2(&f.replace('\\', "/"))
    }

    fn drawtext(&self, body: &str, pos: Pos, size: f32, color: &str, boxed: bool, box_opacity: u8, margin: u32, extra: &str) -> String {
        let (x, y) = text_xy(pos, margin);
        let mut s = format!("drawtext=fontfile={}:{body}:fontsize=h*{:.2}/1080:fontcolor={}:x={x}:y={y}", self.font_arg(), size.max(4.0), color_arg(color, None));
        if boxed {
            s.push_str(&format!(":box=1:boxcolor={}:boxborderw=8", color_arg("#000000", Some(box_opacity))));
        }
        s.push_str(extra);
        s
    }

    /// Filtres de la chaîne principale (avant le logo).
    fn chain(&self) -> Vec<String> {
        let mut f: Vec<String> = Vec::new();
        if self.deinterlace {
            f.push("yadif".into());
        }
        f.push(match self.denoise { 1 => "hqdn3d=1.5:1.2:3:2.5", 2 => "hqdn3d=3:2.5:5:4", 3 => "hqdn3d=6:5:8:7", _ => "" }.into());
        f.retain(|x| !x.is_empty());
        match (self.max_w, self.max_h) {
            (0, 0) => {}
            (w, 0) => f.push(format!("scale='min(iw,{w})':-2:flags=lanczos")),
            (0, h) => f.push(format!("scale=-2:'min(ih,{h})':flags=lanczos")),
            (w, h) => f.push(format!("scale={w}:{h}:force_original_aspect_ratio=decrease:force_divisible_by=2:flags=lanczos")),
        }
        if self.fps > 0.0 {
            f.push(format!("fps={}", trim_f(self.fps)));
        }
        if self.timecode.enabled {
            let t = &self.timecode;
            let body = match t.mode {
                1 => format!("text={}", esc2("%{pts\\:hms}")),
                2 => format!("text={}", esc2("%{frame_num}")) + ":start_number=0",
                _ => {
                    let mut tc = t.start.trim().replace('.', ":");
                    if tc.is_empty() {
                        tc = "00:00:00:00".into();
                    }
                    if t.drop_frame {
                        // Drop-frame : le séparateur des images est « ; ».
                        if let Some(i) = tc.rfind(':') {
                            tc.replace_range(i..i + 1, ";");
                        }
                    }
                    let rate = if t.rate.trim().is_empty() { "auto".to_string() } else { t.rate.trim().to_string() };
                    format!("timecode={}:rate={}", esc2(&tc), if rate == "auto" { "{FPS}".into() } else { esc2(&rate) })
                }
            };
            f.push(self.drawtext(&body, t.pos, t.size, &t.color, t.boxed, t.box_opacity, t.margin, ""));
        }
        for t in &self.texts {
            if t.text.trim().is_empty() {
                continue;
            }
            let en = if t.end > t.start { format!(":enable={}", esc2(&format!("between(t,{},{})", trim_f(t.start), trim_f(t.end)))) } else { String::new() };
            f.push(self.drawtext(&format!("text={}", text_arg(&t.text)), t.pos, t.size, &t.color, t.boxed, t.box_opacity, t.margin, &en));
        }
        f
    }

    pub fn graph(&self) -> Graph {
        let chain = self.chain();
        let logo = self.logo.as_ref().filter(|l| !l.path.trim().is_empty());
        match logo {
            None => Graph { inputs: vec![], vf: chain.join(","), complex: None },
            Some(l) => {
                let base = if chain.is_empty() { "null".to_string() } else { chain.join(",") };
                let (x, y) = overlay_xy(l.pos, l.margin);
                let alpha = if l.opacity < 100 { format!(",colorchannelmixer=aa={:.2}", l.opacity as f32 / 100.0) } else { String::new() };
                let g = format!(
                    "[0:v]{base}[b];[1:v]format=rgba{alpha}[lg];[lg][b]scale2ref=w=main_w*{:.2}/100:h=ow/a[lg2][b2];[b2][lg2]overlay={x}:{y}:format=auto[v]",
                    l.width_pct.clamp(1.0, 100.0)
                );
                Graph { inputs: vec![l.path.trim().to_string()], vf: String::new(), complex: Some(g) }
            }
        }
    }

    /// Arguments FFmpeg (à placer entre l'entrée et la sortie).
    pub fn to_args(&self) -> Vec<String> {
        let mut a: Vec<String> = Vec::new();
        let push = |a: &mut Vec<String>, v: &[&str]| a.extend(v.iter().map(|s| s.to_string()));
        let g = self.graph();
        for p in &g.inputs {
            push(&mut a, &["-i"]);
            a.push(p.clone());
        }
        let copy = self.vcodec == "copy";
        let complex = g.complex.is_some() && !copy;
        if self.all_tracks || complex {
            push(&mut a, &["-map", if complex { "[v]" } else { "0:v:0" }]);
            if self.acodec != "none" {
                push(&mut a, &["-map", if self.all_tracks { "0:a?" } else { "0:a:0?" }]);
            }
            if self.all_tracks && self.container != "webm" {
                push(&mut a, &["-map", "0:s?"]);
            }
        }
        if copy {
            push(&mut a, &["-c:v", "copy"]);
        } else {
            let (enc, tenbit_fmt) = match self.vcodec.as_str() {
                "h265" => ("libx265", "yuv420p10le"),
                "vp9" => ("libvpx-vp9", "yuv420p10le"),
                "av1" => ("libsvtav1", "yuv420p10le"),
                "prores" => ("prores_ks", "yuv422p10le"),
                _ => ("libx264", "yuv420p10le"),
            };
            push(&mut a, &["-c:v", enc]);
            match self.vcodec.as_str() {
                "prores" => push(&mut a, &["-profile:v", "3"]),
                "vp9" => {
                    if self.bitrate_mode { a.extend(["-b:v".into(), format!("{}k", self.vbitrate)]) } else { a.extend(["-crf".into(), self.crf.to_string(), "-b:v".into(), "0".into()]) }
                    push(&mut a, &["-row-mt", "1", "-cpu-used", "2"]);
                }
                "av1" => {
                    push(&mut a, &["-preset", "6"]);
                    if self.bitrate_mode { a.extend(["-b:v".into(), format!("{}k", self.vbitrate)]) } else { a.extend(["-crf".into(), self.crf.to_string()]) }
                }
                _ => {
                    a.extend(["-preset".into(), self.preset.clone()]);
                    if self.bitrate_mode { a.extend(["-b:v".into(), format!("{}k", self.vbitrate)]) } else { a.extend(["-crf".into(), self.crf.to_string()]) }
                    if !self.tune.is_empty() {
                        a.extend(["-tune".into(), self.tune.clone()]);
                    }
                    if self.vcodec == "h264" {
                        a.extend(["-profile:v".into(), if self.tenbit { "high10" } else { "high" }.into()]);
                    }
                }
            }
            if self.vcodec == "prores" {
                push(&mut a, &["-pix_fmt", "yuv422p10le"]);
            } else {
                a.extend(["-pix_fmt".into(), if self.tenbit { tenbit_fmt } else { "yuv420p" }.into()]);
            }
            if self.vcodec == "h265" && matches!(self.container.as_str(), "mp4" | "mov") {
                push(&mut a, &["-tag:v", "hvc1"]);
            }
            if !g.vf.is_empty() && !complex {
                a.extend(["-vf".into(), g.vf.clone()]);
            }
            if let Some(c) = &g.complex {
                a.extend(["-filter_complex".into(), c.clone()]);
            }
        }
        match self.acodec.as_str() {
            "none" => push(&mut a, &["-an"]),
            "copy" => push(&mut a, &["-c:a", "copy"]),
            c => {
                let enc = match c { "mp3" => "libmp3lame", "opus" => "libopus", "ac3" => "ac3", "flac" => "flac", _ => "aac" };
                a.extend(["-c:a".into(), enc.into()]);
                if c != "flac" {
                    a.extend(["-b:a".into(), format!("{}k", self.abitrate)]);
                }
                if self.channels > 0 {
                    a.extend(["-ac".into(), self.channels.to_string()]);
                }
            }
        }
        if self.all_tracks && self.container != "webm" {
            a.extend(["-c:s".into(), if self.container == "mkv" { "copy" } else { "mov_text" }.into()]);
        }
        if self.faststart && matches!(self.container.as_str(), "mp4" | "mov") {
            push(&mut a, &["-movflags", "+faststart"]);
        }
        if self.container == "mkv" {
            push(&mut a, &["-max_muxing_queue_size", "1024"]);
        }
        a.extend(crate::split_args(&self.extra));
        a
    }

    /// Arguments pour une image d'aperçu (`input` : vidéo source, 1 image à `at` secondes, PPM sur la sortie standard).
    pub fn preview_args(&self, input: &str, at: f64, fps: &str) -> Vec<String> {
        let g = self.graph();
        let mut a: Vec<String> = ["-hide_banner", "-nostdin", "-y", "-ss"].iter().map(|s| s.to_string()).collect();
        a.push(format!("{at:.3}"));
        a.push("-i".into());
        a.push(input.into());
        for p in &g.inputs {
            a.extend(["-i".into(), p.clone()]);
        }
        match &g.complex {
            Some(c) => a.extend(["-filter_complex".into(), c.replace("{FPS}", fps), "-map".into(), "[v]".into()]),
            None if !g.vf.is_empty() => a.extend(["-vf".into(), g.vf.replace("{FPS}", fps)]),
            None => {}
        }
        a.extend(["-frames:v", "1", "-an", "-f", "image2pipe", "-c:v", "ppm", "pipe:1"].iter().map(|s| s.to_string()));
        a
    }
}

fn trim_f(v: f32) -> String {
    let s = format!("{v:.3}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_h264() {
        let a = VideoSpec::default().to_args().join(" ");
        assert!(a.contains("-c:v libx264 -preset medium -crf 23 -profile:v high -pix_fmt yuv420p"));
        assert!(a.contains("-c:a aac -b:a 160k") && a.contains("+faststart"));
        assert!(!a.contains("-vf"));
    }

    #[test]
    fn hdlight_like() {
        let s = VideoSpec { container: "mkv".into(), vcodec: "h265".into(), crf: 23, tenbit: true, max_h: 1080, all_tracks: true, acodec: "aac".into(), abitrate: 256, ..Default::default() };
        let a = s.to_args().join(" ");
        assert!(a.contains("-map 0:v:0 -map 0:a? -map 0:s?"));
        assert!(a.contains("-c:v libx265") && a.contains("yuv420p10le") && a.contains("-c:s copy"));
        assert!(a.contains("scale=-2:'min(ih,1080)':flags=lanczos"));
        assert_eq!(s.extension(), "mkv");
    }

    #[test]
    fn timecode_and_text_escaping() {
        let mut s = VideoSpec::default();
        s.timecode.enabled = true;
        s.timecode.drop_frame = true;
        s.texts.push(TextOverlay { text: "Rush: 50% d'essai, ok".into(), ..Default::default() });
        let g = s.graph();
        assert!(g.vf.contains(r"timecode=00\\:00\\:00\;00:rate={FPS}"), "{}", g.vf);
        // ':' ',' '\'' et '%' doivent être neutralisés pour le texte libre
        assert!(g.vf.contains(r"text=Rush\\: 50\\\\%"), "{}", g.vf);
        assert!(g.vf.contains(r"d\\\'essai\, ok"), "{}", g.vf);
        assert!(g.vf.contains("drawtext=fontfile="));
    }

    #[test]
    fn logo_uses_filter_complex() {
        let mut s = VideoSpec::default();
        s.logo = Some(LogoOverlay { path: "C:/logo.png".into(), opacity: 80, ..Default::default() });
        s.max_h = 720;
        let a = s.to_args();
        assert_eq!(&a[..2], ["-i", "C:/logo.png"]);
        let j = a.join(" ");
        assert!(j.contains("-map [v]") && j.contains("-filter_complex [0:v]scale=-2:'min(ih,720)'"), "{j}");
        assert!(j.contains("scale2ref") && j.contains("overlay=W-w-24:24"));
        assert!(!j.contains("-vf"));
    }

    #[test]
    fn problems_detected() {
        let mut s = VideoSpec { vcodec: "copy".into(), max_h: 720, ..Default::default() };
        assert!(!s.problems().is_empty());
        s.vcodec = "h264".into();
        s.container = "webm".into();
        assert!(s.problems().len() >= 2);
    }

    #[test]
    fn roundtrip_json() {
        let s = VideoSpec { logo: Some(LogoOverlay::default()), ..Default::default() };
        let j = serde_json::to_string(&s).unwrap();
        assert_eq!(serde_json::from_str::<VideoSpec>(&j).unwrap(), s);
        assert_eq!(serde_json::from_str::<VideoSpec>("{}").unwrap(), VideoSpec::default());
    }
}
