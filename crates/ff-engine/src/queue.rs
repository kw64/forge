//! File d'attente : N workers, annulation, événements. Indépendante de l'interface.
use crate::{run, Binaries, Cancelled};
use ff_core::{Event, Job, JobId};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};

const MAX_WORKERS: usize = 8;

struct State {
    pending: VecDeque<(JobId, Job)>,
    flags: HashMap<JobId, Arc<AtomicBool>>,
    running: usize,
    max_parallel: usize,
    shutdown: bool,
    next_id: JobId,
}

struct Shared {
    state: Mutex<State>,
    cv: Condvar,
}

/// File d'attente de conversions. Les événements arrivent sur le `Receiver` retourné par `new`.
pub struct Queue {
    shared: Arc<Shared>,
}

impl Queue {
    /// `notify` est appelé après chaque événement (ex. demande de redessin de l'interface).
    pub fn new(
        bins: Binaries,
        max_parallel: usize,
        notify: Arc<dyn Fn() + Send + Sync>,
    ) -> (Self, Receiver<Event>) {
        let (tx, rx) = channel::<Event>();
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                pending: VecDeque::new(),
                flags: HashMap::new(),
                running: 0,
                max_parallel: max_parallel.clamp(1, MAX_WORKERS),
                shutdown: false,
                next_id: 1,
            }),
            cv: Condvar::new(),
        });
        for _ in 0..MAX_WORKERS {
            let (shared, bins, tx, notify) = (shared.clone(), bins.clone(), tx.clone(), notify.clone());
            std::thread::spawn(move || worker(shared, bins, tx, notify));
        }
        (Self { shared }, rx)
    }

    /// Ajoute une tâche ; retourne son identifiant.
    pub fn enqueue(&self, job: Job) -> JobId {
        let mut st = self.shared.state.lock().unwrap();
        let id = st.next_id;
        st.next_id += 1;
        st.flags.insert(id, Arc::new(AtomicBool::new(false)));
        st.pending.push_back((id, job));
        self.shared.cv.notify_all();
        id
    }

    /// Annule une tâche en attente ou en cours.
    pub fn cancel(&self, id: JobId) {
        if let Some(f) = self.shared.state.lock().unwrap().flags.get(&id) {
            f.store(true, Ordering::Relaxed);
        }
        self.shared.cv.notify_all();
    }

    /// Annule tout.
    pub fn cancel_all(&self) {
        for f in self.shared.state.lock().unwrap().flags.values() {
            f.store(true, Ordering::Relaxed);
        }
        self.shared.cv.notify_all();
    }

    /// Change le nombre de conversions simultanées (1 à 8).
    pub fn set_parallelism(&self, n: usize) {
        self.shared.state.lock().unwrap().max_parallel = n.clamp(1, MAX_WORKERS);
        self.shared.cv.notify_all();
    }
}

impl Drop for Queue {
    fn drop(&mut self) {
        let mut st = self.shared.state.lock().unwrap();
        st.shutdown = true;
        for f in st.flags.values() {
            f.store(true, Ordering::Relaxed);
        }
        self.shared.cv.notify_all();
        // Laisse aux workers le temps de tuer les processus ffmpeg et de supprimer les sorties partielles.
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(2000);
        while st.running > 0 {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                break;
            }
            st = self.shared.cv.wait_timeout(st, left).unwrap().0;
        }
    }
}

fn worker(shared: Arc<Shared>, bins: Binaries, tx: Sender<Event>, notify: Arc<dyn Fn() + Send + Sync>) {
    let send = |e: Event| {
        let _ = tx.send(e);
        notify();
    };
    loop {
        let (id, job, flag) = {
            let mut st = shared.state.lock().unwrap();
            loop {
                if st.shutdown {
                    return;
                }
                if !st.pending.is_empty() && st.running < st.max_parallel {
                    break;
                }
                st = shared.cv.wait(st).unwrap();
            }
            let (id, job) = st.pending.pop_front().unwrap();
            let flag = st.flags.get(&id).cloned().unwrap_or_default();
            st.running += 1;
            (id, job, flag)
        };

        if flag.load(Ordering::Relaxed) {
            send(Event::Cancelled(id));
        } else {
            send(Event::Started(id));
            let result = run(&bins, &job, flag.clone(), |p| send(Event::Progress(id, p)), |m| send(Event::Info(id, m)));
            match result {
                Ok(()) => send(Event::Done(id)),
                Err(e) if e.downcast_ref::<Cancelled>().is_some() => send(Event::Cancelled(id)),
                Err(e) => send(Event::Failed(id, format!("{e:#}"))),
            }
        }

        let mut st = shared.state.lock().unwrap();
        st.running -= 1;
        st.flags.remove(&id);
        shared.cv.notify_all();
    }
}
