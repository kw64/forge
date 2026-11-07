//! Test d'intégration : nécessite ffmpeg/ffprobe (ignoré s'ils sont absents).
use ff_core::{builtin_by_id, Crop, Edits, Event, HwAccel, Job, JobKind};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn wait_terminal(rx: &std::sync::mpsc::Receiver<Event>, want: usize) -> Vec<Event> {
    let (mut out, t0) = (Vec::new(), Instant::now());
    while out.len() < want && t0.elapsed() < Duration::from_secs(120) {
        if let Ok(e) = rx.recv_timeout(Duration::from_millis(200)) {
            if matches!(e, Event::Done(_) | Event::Failed(..) | Event::Cancelled(_)) {
                out.push(e);
            }
        }
    }
    out
}

#[test]
fn queue_runs_fails_and_cancels() {
    let Ok(bins) = ff_engine::locate(None) else { eprintln!("ffmpeg absent : test ignoré"); return };
    let dir = std::env::temp_dir().join(format!("mf_q_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("in.mkv");
    let ok = Command::new(&bins.ffmpeg)
        .args(["-loglevel", "error", "-y", "-f", "lavfi", "-i", "testsrc=duration=2:size=320x240:rate=25", "-f", "lavfi", "-i", "sine=duration=2", "-shortest"])
        .arg(&src).stdout(Stdio::null()).status().unwrap().success();
    assert!(ok);

    let (q, rx) = ff_engine::Queue::new(bins, 2, Arc::new(|| {}));
    let mk = |name: &str, input: std::path::PathBuf| Job::convert(input, dir.join(name), builtin_by_id("mp3").unwrap());
    let good = q.enqueue(mk("a.mp3", src.clone()));
    let bad = q.enqueue(mk("b.mp3", dir.join("n_existe_pas.mkv")));
    let evs = wait_terminal(&rx, 2);
    assert!(evs.iter().any(|e| matches!(e, Event::Done(i) if *i == good)), "{evs:?}");
    assert!(evs.iter().any(|e| matches!(e, Event::Failed(i, _) if *i == bad)), "{evs:?}");
    assert!(dir.join("a.mp3").exists());

    // annulation avant démarrage : parallélisme mis à 1, deux tâches, la 2e annulée tout de suite
    q.set_parallelism(1);
    let _first = q.enqueue(mk("c.mp3", src.clone()));
    let second = q.enqueue(mk("d.mp3", src.clone()));
    q.cancel(second);
    let evs = wait_terminal(&rx, 2);
    assert!(evs.iter().any(|e| matches!(e, Event::Cancelled(i) if *i == second)), "{evs:?}");
    assert!(!dir.join("d.mp3").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

fn probe_dur(bins: &ff_engine::Binaries, p: &std::path::Path) -> f64 {
    ff_probe::probe(&bins.ffprobe, p).unwrap().duration_secs.unwrap_or(0.0)
}

fn run_job(bins: &ff_engine::Binaries, job: &Job) -> (anyhow::Result<()>, Vec<String>) {
    let mut infos = Vec::new();
    let r = ff_engine::run(bins, job, Arc::new(std::sync::atomic::AtomicBool::new(false)), |_| {}, |m| infos.push(m));
    (r, infos)
}

#[test]
fn v1_features_trim_crop_image_tools_hw() {
    let Ok(bins) = ff_engine::locate(None) else { eprintln!("ffmpeg absent : test ignoré"); return };
    let dir = std::env::temp_dir().join(format!("mf_v1_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mk = |name: &str, spec: &str, secs: u32| {
        let p = dir.join(name);
        let ok = Command::new(&bins.ffmpeg)
            .args(["-loglevel", "error", "-y", "-f", "lavfi", "-i", &format!("{spec}=duration={secs}:size=320x240:rate=25"), "-f", "lavfi", "-i", &format!("sine=duration={secs}"), "-shortest", "-pix_fmt", "yuv420p"])
            .arg(&p).stdout(Stdio::null()).status().unwrap().success();
        assert!(ok);
        p
    };
    let v1 = mk("v1.mkv", "testsrc", 6);
    let v2 = mk("v2.mkv", "testsrc2", 4);
    let p = |id: &str| builtin_by_id(id).unwrap();

    // découpe + recadrage
    let out = dir.join("trim.mp4");
    let mut j = Job::convert(v1.clone(), out.clone(), p("mp4"));
    j.edits = Edits { trim_start: Some(1.0), trim_end: Some(3.0), crop: Some(Crop { w: 160, h: 120, x: 0, y: 0 }) };
    let (r, _) = run_job(&bins, &j);
    r.unwrap();
    let d = probe_dur(&bins, &out);
    assert!((d - 2.0).abs() < 0.3, "durée {d}");
    let info = Command::new(&bins.ffprobe).args(["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=width,height", "-of", "csv=p=0"]).arg(&out).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&info.stdout).trim(), "160,120");

    // image : capture d'une image de la vidéo, dans plusieurs formats
    for id in ["jpg", "png", "webp", "bmp", "tiff"] {
        let o = dir.join(format!("shot.{id}"));
        let mut j = Job::convert(v1.clone(), o.clone(), p(id));
        j.edits.trim_start = Some(2.0);
        run_job(&bins, &j).0.unwrap_or_else(|e| panic!("{id}: {e:#}"));
        assert!(o.metadata().unwrap().len() > 500, "{id}");
    }

    // fusion avec réencodage, puis sans
    let o = dir.join("fusion.mp4");
    let j = Job { kind: JobKind::Concat { inputs: vec![v1.clone(), v2.clone()], copy: false }, ..Job::convert(v1.clone(), o.clone(), p("mp4")) };
    run_job(&bins, &j).0.unwrap();
    assert!((probe_dur(&bins, &o) - 10.0).abs() < 0.5);
    let o = dir.join("fusion_copy.mkv");
    let j = Job { kind: JobKind::Concat { inputs: vec![v1.clone(), v2.clone()], copy: true }, ..Job::convert(v1.clone(), o.clone(), p("mkv")) };
    run_job(&bins, &j).0.unwrap();
    assert!((probe_dur(&bins, &o) - 10.0).abs() < 0.5);

    // mux : une vidéo + une piste audio
    let a = dir.join("voix.mp3");
    run_job(&bins, &Job::convert(v2.clone(), a.clone(), p("mp3"))).0.unwrap();
    let o = dir.join("mux.mp4");
    let j = Job { kind: JobKind::Mux { audio: a.clone() }, ..Job::convert(v1.clone(), o.clone(), p("mp4")) };
    run_job(&bins, &j).0.unwrap();
    assert!(o.exists());

    // mix de deux audios
    let o = dir.join("mix.mp3");
    let j = Job { kind: JobKind::Mix { inputs: vec![a.clone(), dir.join("fusion.mp4")] }, ..Job::convert(a.clone(), o.clone(), p("mp3")) };
    run_job(&bins, &j).0.unwrap();
    assert!(probe_dur(&bins, &o) > 5.0);

    // accélération matérielle demandée : repli logiciel automatique si le matériel est absent
    let o = dir.join("hw.mp4");
    let mut j = Job::convert(v2.clone(), o.clone(), p("mp4"));
    j.hw = HwAccel::Nvenc;
    let (r, infos) = run_job(&bins, &j);
    r.unwrap();
    assert!(o.exists());
    eprintln!("infos hw : {infos:?}");

    // recadrage refusé sur un profil audio
    let mut j = Job::convert(v1.clone(), dir.join("bad.mp3"), p("mp3"));
    j.edits.crop = Some(Crop { w: 10, h: 10, x: 0, y: 0 });
    assert!(run_job(&bins, &j).0.is_err());
    let _ = std::fs::remove_dir_all(&dir);
}
