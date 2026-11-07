//! Construction pure des arguments d'encodage (accélération matérielle, recadrage). Testable sans FFmpeg.
use crate::{Crop, Edits, HwAccel, Profile};

/// Fusionne un filtre dans l'option `-vf` existante (ou la crée).
fn merge_vf(args: &mut Vec<String>, prepend: Option<&str>, append: Option<&str>) {
    if prepend.is_none() && append.is_none() {
        return;
    }
    let mut chain: Vec<String> = Vec::new();
    if let Some(p) = prepend {
        chain.push(p.to_string());
    }
    let pos = args.iter().position(|a| a == "-vf");
    if let Some(i) = pos {
        if let Some(existing) = args.get(i + 1).cloned() {
            chain.push(existing);
        }
    }
    if let Some(a) = append {
        chain.push(a.to_string());
    }
    let joined = chain.join(",");
    match pos {
        Some(i) if i + 1 < args.len() => args[i + 1] = joined,
        _ => {
            args.push("-vf".into());
            args.push(joined);
        }
    }
}

fn remove_opt(args: &mut Vec<String>, opt: &str, with_value: bool) {
    while let Some(i) = args.iter().position(|a| a == opt) {
        args.remove(i);
        if with_value && i < args.len() {
            args.remove(i);
        }
    }
}

fn replace_opt(args: &mut [String], from: &str, to: &str) {
    for a in args.iter_mut().filter(|a| a.as_str() == from) {
        *a = to.to_string();
    }
}

/// Filtre de recadrage (dimensions arrondies au pair, requis par le YUV 4:2:0).
pub fn crop_filter(c: Crop) -> String {
    format!("crop={}:{}:{}:{}", (c.w & !1).max(2), (c.h & !1).max(2), c.x, c.y)
}

fn copies_video(args: &[String]) -> bool {
    args.windows(2).any(|w| (w[0] == "-c" || w[0] == "-c:v") && w[1] == "copy")
}

/// Arguments de codec d'un profil, avec retouches et accélération matérielle.
/// Retourne `(arguments, accélération_appliquée)`.
pub fn codec_args(profile: &Profile, hw: HwAccel, edits: &Edits) -> Result<(Vec<String>, bool), String> {
    let mut a = profile.args.clone();
    let audio_only = profile.is_audio_only();

    let mut crop_f = None;
    if let Some(c) = edits.crop {
        if audio_only {
            return Err("le recadrage ne s'applique pas à un profil audio".into());
        }
        if copies_video(&a) {
            return Err("le recadrage est impossible avec un profil qui copie les flux (choisissez un profil avec réencodage)".into());
        }
        if c.w < 2 || c.h < 2 {
            return Err("recadrage invalide (largeur et hauteur d'au moins 2 pixels)".into());
        }
        crop_f = Some(crop_filter(c));
    }
    // Graphe complet (logo) : le recadrage s'insère en tête de la chaîne principale.
    let complex = a.iter().position(|x| x == "-filter_complex");
    match (complex, crop_f.as_deref()) {
        (Some(i), Some(c)) => {
            if let Some(g) = a.get_mut(i + 1) {
                *g = g.replacen("[0:v]", &format!("[0:v]{c},"), 1);
            }
        }
        (Some(_), None) => {}
        (None, _) => merge_vf(&mut a, crop_f.as_deref(), None),
    }

    let mut applied = false;
    if hw != HwAccel::None && hw != HwAccel::Auto && !audio_only && !(hw == HwAccel::Vaapi && complex.is_some()) {
        let sfx = hw.suffix();
        for (soft, name) in [("libx264", "h264"), ("libx265", "hevc")] {
            if a.iter().any(|x| x == soft) {
                replace_opt(&mut a, soft, &format!("{name}_{sfx}"));
                applied = true;
            }
        }
        if applied {
            match hw {
                HwAccel::Nvenc => {
                    remove_opt(&mut a, "-preset", true);
                    // NVENC n'a pas la même échelle que x264/x265 : à qualité visuelle égale le CQ est un peu plus haut
                    // (+2 en H.264, +3 en H.265). Approximation à ajuster selon vos goûts via un profil personnalisé.
                    let hevc = a.iter().any(|x| x == "hevc_nvenc");
                    if let Some(i) = a.iter().position(|x| x == "-crf") {
                        a[i] = "-cq".into();
                        if let Some(v) = a.get_mut(i + 1) {
                            if let Ok(n) = v.parse::<i32>() {
                                *v = (n + if hevc { 3 } else { 2 }).to_string();
                            }
                        }
                    }
                    if !a.iter().any(|x| x == "-b:v") {
                        a.push("-b:v".into());
                        a.push("0".into());
                    }
                    // Réglages qualité : preset p6, VBR à qualité constante, 2 passes, AQ spatiale + temporelle.
                    for (k, v) in [("-preset", "p6"), ("-tune", "hq"), ("-rc", "vbr"), ("-multipass", "fullres"), ("-spatial-aq", "1"), ("-temporal-aq", "1"), ("-rc-lookahead", "32")] {
                        a.push(k.into());
                        a.push(v.into());
                    }
                    // H.265 10 bits : profil Main10 explicite.
                    if hevc && a.windows(2).any(|w| w[0] == "-pix_fmt" && w[1].contains("10")) && !a.iter().any(|x| x == "-profile:v") {
                        a.push("-profile:v".into());
                        a.push("main10".into());
                    }
                }
                HwAccel::Qsv => replace_opt(&mut a, "-crf", "-global_quality"),
                HwAccel::Amf => {
                    // AMF : qualité constante via -rc cqp + -qp_i/-qp_p/-qp_b.
                    remove_opt(&mut a, "-preset", true);
                    if let Some(i) = a.iter().position(|x| x == "-crf") {
                        let q = a.get(i + 1).cloned().unwrap_or_else(|| "23".into());
                        a.drain(i..(i + 2).min(a.len()));
                        for (k, v) in [("-rc", "cqp"), ("-qp_i", &q), ("-qp_p", &q), ("-qp_b", &q)] {
                            a.push(k.into());
                            a.push(v.to_string());
                        }
                    }
                }
                HwAccel::Vaapi => {
                    remove_opt(&mut a, "-preset", true);
                    replace_opt(&mut a, "-crf", "-qp");
                    merge_vf(&mut a, None, Some("format=nv12,hwupload"));
                    a.splice(0..0, ["-vaapi_device".to_string(), "/dev/dri/renderD128".to_string()]);
                }
                HwAccel::None | HwAccel::Auto => {}
            }
        }
    }
    Ok((a, applied))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtin_by_id;

    #[test]
    fn hdlight_profiles_hw_and_crop() {
        for id in ["hdlight-1080-x264", "hdlight-1080-x265", "hdlight-720-x264", "hdlight-720-x265", "hdlight-mini"] {
            let p = builtin_by_id(id).unwrap();
            assert_eq!(p.extension, "mkv");
            let (a, ok) = codec_args(&p, HwAccel::Nvenc, &Edits::default()).unwrap();
            assert!(ok && a.iter().any(|x| x.ends_with("_nvenc")) && !a.contains(&"-crf".to_string()), "{id}");
            if a.iter().any(|x| x == "hevc_nvenc") {
                assert!(a.windows(2).any(|w| w[0] == "-profile:v" && w[1] == "main10"), "{id}");
                assert!(a.windows(2).any(|w| w[0] == "-pix_fmt" && w[1] == "yuv420p10le"), "{id}");
            }
            let (a, ok) = codec_args(&p, HwAccel::None, &Edits { crop: Some(Crop { x: 0, y: 0, w: 100, h: 100 }), ..Default::default() }).unwrap();
            assert!(!ok);
            let vf = &a[a.iter().position(|x| x == "-vf").unwrap() + 1];
            assert!(vf.starts_with("crop=") && vf.contains("scale="), "{vf}");
        }
    }

    #[test]
    fn crop_merges_into_existing_vf() {
        let p = builtin_by_id("mp4-small").unwrap();
        let e = Edits { crop: Some(Crop { w: 641, h: 361, x: 10, y: 20 }), ..Default::default() };
        let (a, _) = codec_args(&p, HwAccel::None, &e).unwrap();
        let i = a.iter().position(|x| x == "-vf").unwrap();
        assert!(a[i + 1].starts_with("crop=640:360:10:20,scale="), "{}", a[i + 1]);
    }

    #[test]
    fn crop_rejected_on_copy_and_audio() {
        let e = Edits { crop: Some(Crop { w: 100, h: 100, x: 0, y: 0 }), ..Default::default() };
        assert!(codec_args(&builtin_by_id("mkv").unwrap(), HwAccel::None, &e).is_err());
        assert!(codec_args(&builtin_by_id("mp3").unwrap(), HwAccel::None, &e).is_err());
    }

    #[test]
    fn hw_translation() {
        let p = builtin_by_id("mp4").unwrap();
        let (a, ok) = codec_args(&p, HwAccel::Nvenc, &Edits::default()).unwrap();
        assert!(ok && a.contains(&"h264_nvenc".to_string()) && a.contains(&"-cq".to_string()));
        // preset x264 (« medium ») remplacé par le preset NVENC p6, et CQ = CRF + 2.
        assert!(a.windows(2).any(|w| w[0] == "-preset" && w[1] == "p6") && !a.contains(&"medium".to_string()));
        assert!(a.windows(2).any(|w| w[0] == "-cq" && w[1] == "25"));
        let (a, ok) = codec_args(&p, HwAccel::Vaapi, &Edits::default()).unwrap();
        assert!(ok && a[0] == "-vaapi_device" && a.iter().any(|x| x.contains("hwupload")));
        // profil audio : jamais d'accélération
        let (_, ok) = codec_args(&builtin_by_id("mp3").unwrap(), HwAccel::Nvenc, &Edits::default()).unwrap();
        assert!(!ok);
    }

    #[test]
    fn span() {
        let e = Edits { trim_start: Some(10.0), trim_end: Some(25.0), crop: None };
        assert_eq!(e.span(Some(100.0)), Some(15.0));
        let e = Edits { trim_start: Some(10.0), ..Default::default() };
        assert_eq!(e.span(Some(100.0)), Some(90.0));
    }
}
