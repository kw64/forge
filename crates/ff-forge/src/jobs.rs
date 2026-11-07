//! Tâches de fond de FileForge : progression, annulation, file d'attente FIFO (copie/déplacement/suppression/compression/extraction). v0.14.0
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

type Work = Box<dyn FnOnce(&Job) -> Result<String, String> + Send>;

/// Types de tâches passant par la file d'attente séquentielle commune.
pub fn queued(key: &str) -> bool {
    matches!(key, "copy" | "move" | "delete" | "pack" | "unpack")
}

pub struct Job {
    pub key: &'static str,
    pub label: String,
    done: AtomicU64,
    total: AtomicU64,
    cancel: AtomicBool,
    finished: AtomicBool,
    started: AtomicBool,
    work: Mutex<Option<Work>>,
    detail: Mutex<String>,
    result: Mutex<Option<Result<String, String>>>,
}

impl Job {
    /// Total attendu (octets ou éléments) ; 0 = indéterminé.
    pub fn set_total(&self, n: u64) {
        self.total.store(n, Ordering::Relaxed);
    }
    /// Avance de `n` ; renvoie `false` si l'utilisateur a demandé l'annulation.
    pub fn step(&self, n: u64) -> bool {
        self.done.fetch_add(n, Ordering::Relaxed);
        !self.cancelled()
    }
    /// Nouvelle phase : remet la progression à zéro avec un nouveau total.
    pub fn reset(&self, total: u64) {
        self.done.store(0, Ordering::Relaxed);
        self.total.store(total, Ordering::Relaxed);
    }
    pub fn set_detail(&self, s: impl Into<String>) {
        if let Ok(mut d) = self.detail.lock() {
            *d = s.into();
        }
    }
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
    /// Fraction 0..1 (None si indéterminé).
    pub fn fraction(&self) -> Option<f32> {
        let t = self.total.load(Ordering::Relaxed);
        (t > 0).then(|| (self.done.load(Ordering::Relaxed) as f32 / t as f32).clamp(0.0, 1.0))
    }
    /// En attente dans la file (pas encore démarrée).
    pub fn pending(&self) -> bool {
        !self.started.load(Ordering::SeqCst) && !self.finished.load(Ordering::SeqCst)
    }
    pub fn detail(&self) -> String {
        self.detail.lock().map(|d| d.clone()).unwrap_or_default()
    }
}

/// Entrée d'historique : (libellé, succès, message).
pub type Done = (String, bool, String);

#[derive(Default)]
pub struct Jobs {
    list: Vec<Arc<Job>>,
    pub history: Vec<Done>,
}

impl Jobs {
    /// Ajoute une tâche. Les types « file » sont mis en attente (FIFO, une à la fois) ;
    /// les autres démarrent aussitôt (refus si le même type tourne déjà).
    pub fn spawn(&mut self, key: &'static str, label: impl Into<String>, f: impl FnOnce(&Job) -> Result<String, String> + Send + 'static) -> bool {
        let q = queued(key);
        if !q && self.running(key) {
            return false;
        }
        let job = Arc::new(Job { key, label: label.into(), done: AtomicU64::new(0), total: AtomicU64::new(0), cancel: AtomicBool::new(false), finished: AtomicBool::new(false), started: AtomicBool::new(false), detail: Mutex::new(String::new()), work: Mutex::new(Some(Box::new(f))), result: Mutex::new(None) });
        self.list.push(job);
        self.pump();
        true
    }

    fn start(job: Arc<Job>) {
        job.started.store(true, Ordering::SeqCst);
        std::thread::spawn(move || {
            let w = job.work.lock().ok().and_then(|mut g| g.take());
            let r = match w {
                // Une panique ne doit pas laisser la tâche « en cours » indéfiniment.
                Some(f) => std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(&job))).unwrap_or_else(|_| Err("erreur interne".into())),
                None => Err("?".into()),
            };
            if let Ok(mut g) = job.result.lock() {
                *g = Some(r);
            }
            job.finished.store(true, Ordering::SeqCst);
        });
    }

    /// Démarre les tâches non-file en attente et la prochaine de la file si la voie est libre.
    pub fn pump(&mut self) {
        let lane_busy = self.list.iter().any(|j| queued(j.key) && j.started.load(Ordering::SeqCst) && !j.finished.load(Ordering::SeqCst));
        let mut lane = lane_busy;
        for j in self.list.clone() {
            if j.started.load(Ordering::SeqCst) || j.finished.load(Ordering::SeqCst) {
                continue;
            }
            if queued(j.key) {
                if lane {
                    continue;
                }
                lane = true;
            }
            Self::start(j);
        }
    }

    /// Annule une tâche : en attente → retirée immédiatement ; en cours → arrêt demandé.
    pub fn cancel(&mut self, job: &Arc<Job>) {
        if job.pending() {
            job.cancel();
            if let Ok(mut g) = job.work.lock() {
                *g = None;
            }
            if let Ok(mut g) = job.result.lock() {
                *g = Some(Err("Annulé".into()));
            }
            job.finished.store(true, Ordering::SeqCst);
        } else {
            job.cancel();
        }
    }

    /// Annule toutes les tâches de la file.
    pub fn cancel_all(&mut self) {
        for j in self.list.clone() {
            if queued(j.key) {
                self.cancel(&j);
            }
        }
    }

    /// Passe une tâche en attente avant les autres (tête de file).
    pub fn move_up(&mut self, job: &Arc<Job>) {
        if let Some(i) = self.list.iter().position(|j| Arc::ptr_eq(j, job)) {
            if let Some(p) = (0..i).rev().find(|&p| self.list[p].pending() && queued(self.list[p].key)) {
                if job.pending() {
                    self.list.swap(i, p);
                }
            }
        }
    }

    pub fn running(&self, key: &str) -> bool {
        self.list.iter().any(|j| j.key == key && !j.finished.load(Ordering::SeqCst))
    }

    pub fn active(&self) -> &[Arc<Job>] {
        &self.list
    }

    pub fn any(&self) -> bool {
        !self.list.is_empty()
    }

    /// Récupère les tâches terminées (retirées de la liste) avec leur résultat.
    pub fn poll(&mut self) -> Vec<(Arc<Job>, Result<String, String>)> {
        let mut out = Vec::new();
        self.list.retain(|j| {
            if j.finished.load(Ordering::SeqCst) {
                let r = j.result.lock().ok().and_then(|mut g| g.take()).unwrap_or_else(|| Err("?".into()));
                out.push((j.clone(), r));
                false
            } else {
                true
            }
        });
        for (j, r) in &out {
            if queued(j.key) {
                self.history.push((j.label.clone(), r.is_ok(), match r { Ok(m) | Err(m) => m.clone() }));
                if self.history.len() > 50 {
                    self.history.remove(0);
                }
            }
        }
        self.pump();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn queue_is_sequential_and_cancellable() {
        let mut j = Jobs::default();
        j.spawn("copy", "a", |_| { std::thread::sleep(Duration::from_millis(150)); Ok("a".into()) });
        j.spawn("move", "b", |_| Ok("b".into()));
        j.spawn("delete", "c", |_| Ok("c".into()));
        assert!(!j.active()[0].pending() && j.active()[1].pending() && j.active()[2].pending());
        let c = j.active()[2].clone();
        j.cancel(&c);
        let mut order = vec![];
        for _ in 0..100 {
            for (jb, r) in j.poll() {
                order.push((jb.label.clone(), r.is_ok()));
            }
            if !j.any() { break; }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(order, vec![("c".into(), false), ("a".into(), true), ("b".into(), true)]);
    }
}
