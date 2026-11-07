//! Profil visuel : encodage réel avec timecode, texte (caractères spéciaux) et logo. Ignoré sans ffmpeg complet.
use ff_core::spec::{LogoOverlay, TextOverlay, TimecodeOverlay, VideoSpec};
use ff_core::{Event, Job, Profile};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[test]
fn visual_profile_encodes_with_overlays() {
    let Ok(bins) = ff_engine::locate(None) else { eprintln!("ffmpeg absent : test ignoré"); return };
    let enc = ff_engine::available_encoders(&bins);
    if !enc.contains("libx264") {
        eprintln!("libx264 absent : test ignoré");
        return;
    }
    let dir = std::env::temp_dir().join(format!("mf_spec_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (src, logo) = (dir.join("in.mkv"), dir.join("logo.png"));
    let run = |a: &[&str], out: &std::path::Path| Command::new(&bins.ffmpeg).args(["-loglevel", "error", "-y"]).args(a).arg(out).stdout(Stdio::null()).status().unwrap().success();
    assert!(run(&["-f", "lavfi", "-i", "testsrc=duration=2:size=640x360:rate=30000/1001", "-f", "lavfi", "-i", "sine=duration=2", "-shortest"], &src));
    assert!(run(&["-f", "lavfi", "-i", "color=c=red:s=120x60", "-frames:v", "1"], &logo));

    let spec = VideoSpec {
        max_h: 240,
        crf: 30,
        preset: "ultrafast".into(),
        timecode: TimecodeOverlay { enabled: true, drop_frame: true, ..Default::default() },
        texts: vec![TextOverlay { text: "Rush: 50% d'essai, [ok]".into(), end: 1.0, ..Default::default() }],
        logo: Some(LogoOverlay { path: logo.display().to_string(), opacity: 80, ..Default::default() }),
        font: std::env::var("MF_TEST_FONT").unwrap_or_default(),
        ..Default::default()
    };
    let profile = Profile { id: "v".into(), label: "v".into(), extension: spec.extension().into(), args: spec.to_args(), builtin: false, spec: Some(spec) };
    let out = dir.join("out.mp4");
    let (q, rx) = ff_engine::Queue::new(bins.clone(), 1, Arc::new(|| {}));
    let id = q.enqueue(Job::convert(src.clone(), out.clone(), profile));
    let t0 = Instant::now();
    let mut res = None;
    while t0.elapsed() < Duration::from_secs(60) && res.is_none() {
        if let Ok(e) = rx.recv_timeout(Duration::from_millis(200)) {
            match e {
                Event::Done(i) if i == id => res = Some(Ok(())),
                Event::Failed(i, m) if i == id => res = Some(Err(m)),
                _ => {}
            }
        }
    }
    assert!(matches!(res, Some(Ok(()))), "{res:?}");
    assert!(std::fs::metadata(&out).unwrap().len() > 1000);
    let info = ff_probe::probe(&bins.ffprobe, &out).unwrap();
    assert_eq!(info.size.map(|s| s.1), Some(240));
}
