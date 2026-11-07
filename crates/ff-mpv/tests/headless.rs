//! Test sans fenêtre : charge un média, lit la durée, fait un seek. Ignoré si libmpv ou ffmpeg manquent.
use ff_mpv::{EndReason, Event, Mpv};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn load_seek_and_query() {
    if ff_mpv::library_available().is_err() {
        eprintln!("libmpv absente : test ignoré");
        return;
    }
    let dir = std::env::temp_dir().join(format!("mf_mpv_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("t.mkv");
    let ok = Command::new("ffmpeg")
        .args(["-loglevel", "error", "-y", "-f", "lavfi", "-i", "testsrc=duration=10:size=320x240:rate=25", "-f", "lavfi", "-i", "sine=duration=10", "-shortest"])
        .arg(&f).stdout(Stdio::null()).status().map(|s| s.success()).unwrap_or(false);
    if !ok {
        eprintln!("ffmpeg absent : test ignoré");
        return;
    }
    let mpv = Mpv::new(&[("vo", "null"), ("ao", "null"), ("idle", "yes"), ("keep-open", "yes"), ("terminal", "no"), ("config", "no")]).unwrap();
    mpv.command(&["loadfile", f.to_str().unwrap(), "replace"]).unwrap();
    let t0 = Instant::now();
    let mut loaded = false;
    while t0.elapsed() < Duration::from_secs(10) && !loaded {
        loaded = mpv.wait_event(0.1) == Event::FileLoaded;
    }
    assert!(loaded, "fichier non chargé");
    let d = mpv.get_f64("duration").unwrap();
    assert!((d - 10.0).abs() < 0.2, "durée {d}");
    mpv.command(&["seek", "7", "absolute"]).unwrap();
    std::thread::sleep(Duration::from_millis(500));
    let pos = mpv.get_f64("time-pos").unwrap();
    assert!(pos >= 6.5, "position {pos}");
    mpv.set_property("pause", "yes").unwrap();
    assert_eq!(mpv.get_bool("pause"), Some(true));
    assert_eq!(mpv.get_string("track-list/count").as_deref(), Some("2"));
    // fin de fichier -> EndFile(Eof) ou keep-open
    mpv.set_property("pause", "no").unwrap();
    mpv.command(&["seek", "9.8", "absolute"]).unwrap();
    let t0 = Instant::now();
    let mut eof = false;
    while t0.elapsed() < Duration::from_secs(8) && !eof {
        eof = mpv.get_bool("eof-reached") == Some(true);
        if let Event::EndFile(EndReason::Eof) = mpv.wait_event(0.1) {
            eof = true;
        }
    }
    assert!(eof, "fin de fichier non détectée");
    // option inconnue ignorée, propriété inconnue = erreur
    assert!(mpv.set_property("propriete-inexistante", "1").is_err());
    let _ = std::fs::remove_dir_all(&dir);
}
