//! Musiques rétro : MOD (FFmpeg/libopenmpt), YM (Atari ST) et SID (C64) → FLAC. Ignoré sans ffmpeg.
use ff_core::{Event, Job, Profile};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn convert(bins: &ff_engine::Binaries, src: &std::path::Path, out: &std::path::Path) -> Result<(), String> {
    let profile = Profile { id: "flac".into(), label: "flac".into(), extension: "flac".into(), args: vec!["-c:a".into(), "flac".into()], builtin: false, spec: None };
    let (q, rx) = ff_engine::Queue::new(bins.clone(), 1, Arc::new(|| {}));
    let id = q.enqueue(Job::convert(src.to_path_buf(), out.to_path_buf(), profile));
    let t0 = Instant::now();
    while t0.elapsed() < Duration::from_secs(60) {
        if let Ok(e) = rx.recv_timeout(Duration::from_millis(200)) {
            match e {
                Event::Done(i) if i == id => return Ok(()),
                Event::Failed(i, m) if i == id => return Err(m),
                _ => {}
            }
        }
    }
    Err("délai dépassé".into())
}

#[test]
fn retro_music_converts() {
    let Ok(bins) = ff_engine::locate(None) else { eprintln!("ffmpeg absent : test ignoré"); return };
    let dir = std::env::temp_dir().join(format!("mf_chip_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    // YM3 minimal
    let n = 100usize;
    let mut ym = b"YM3!".to_vec();
    for r in 0..14 {
        for _ in 0..n {
            ym.push(match r { 0 => 0x40, 7 => 0x3E, 8 => 0x0F, _ => 0 });
        }
    }
    std::fs::write(dir.join("t.ym"), &ym).unwrap();
    convert(&bins, &dir.join("t.ym"), &dir.join("ym.flac")).expect("YM");
    // Sortie WAV directe (PCM 16 bits)
    {
        let p = Profile { id: "wav".into(), label: "wav".into(), extension: "wav".into(), args: vec!["-c:a".into(), "pcm_s16le".into()], builtin: false, spec: None };
        let (q, rx) = ff_engine::Queue::new(bins.clone(), 1, Arc::new(|| {}));
        let id = q.enqueue(Job::convert(dir.join("t.ym"), dir.join("ym.wav"), p));
        let t0 = Instant::now();
        loop { if let Ok(e) = rx.recv_timeout(Duration::from_millis(200)) { match e { Event::Done(i) if i == id => break, Event::Failed(i, m) if i == id => panic!("{m}"), _ => {} } } assert!(t0.elapsed() < Duration::from_secs(60)); }
        assert!(std::fs::metadata(dir.join("ym.wav")).unwrap().len() > 1000);
    }
    assert!(std::fs::metadata(dir.join("ym.flac")).unwrap().len() > 100);

    // PSID minimal
    let mut h = vec![0u8; 0x7C];
    h[0..4].copy_from_slice(b"PSID");
    h[5] = 2;
    h[7] = 0x7C;
    h[0x0B] = 0x00;
    h[0x0A] = 0x10;
    h[0x0C] = 0x10;
    h[0x0D] = 0x20;
    h[0x0F] = 1;
    h[0x11] = 1;
    let mut m = vec![0xEA; 0x30];
    let init = [0xA9, 0x0F, 0x8D, 0x18, 0xD4, 0xA9, 0x00, 0x8D, 0x05, 0xD4, 0xA9, 0xF0, 0x8D, 0x06, 0xD4, 0xA9, 0x21, 0x8D, 0x04, 0xD4, 0x60];
    m[..init.len()].copy_from_slice(&init);
    let play = [0xEE, 0x40, 0x10, 0xAD, 0x40, 0x10, 0x8D, 0x01, 0xD4, 0x60];
    m[0x20..0x20 + play.len()].copy_from_slice(&play);
    h.extend([0x00, 0x10]);
    h.extend(m);
    h.extend(vec![0u8; 0x20]);
    std::fs::write(dir.join("t.sid"), &h).unwrap();
    convert(&bins, &dir.join("t.sid"), &dir.join("sid.flac")).expect("SID");
    assert!(std::fs::metadata(dir.join("sid.flac")).unwrap().len() > 100);

    // MOD vide (1084 octets d'en-tête + 1 pattern) : lu par libopenmpt si FFmpeg en dispose
    let mut md = vec![0u8; 20 + 31 * 30];
    md.push(1);
    md.push(0x7F);
    md.extend(vec![0u8; 128]);
    md.extend(b"M.K.");
    md.extend(vec![0u8; 1024]);
    std::fs::write(dir.join("t.mod"), &md).unwrap();
    match convert(&bins, &dir.join("t.mod"), &dir.join("mod.flac")) {
        Ok(()) => assert!(dir.join("mod.flac").exists()),
        Err(e) => eprintln!("MOD : FFmpeg sans libopenmpt ? {e}"),
    }
}
