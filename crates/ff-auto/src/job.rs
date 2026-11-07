//! Tâches de fond d'AutoForge : progression, annulation, une tâche par type à la fois. v0.1.0
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

pub struct Job {
    pub key: &'static str,
    pub label: String,
    done: AtomicU64,
    total: AtomicU64,
    cancel: AtomicBool,
    finished: AtomicBool,
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
    pub fn detail(&self) -> String {
        self.detail.lock().map(|d| d.clone()).unwrap_or_default()
    }
}

#[derive(Default)]
pub struct Jobs {
    list: Vec<Arc<Job>>,
}

impl Jobs {
    /// Lance `f` en arrière-plan. Refuse (false) si une tâche du même type `key` tourne déjà.
    pub fn spawn(&mut self, key: &'static str, label: impl Into<String>, f: impl FnOnce(&Job) -> Result<String, String> + Send + 'static) -> bool {
        if self.running(key) {
            return false;
        }
        let job = Arc::new(Job { key, label: label.into(), done: AtomicU64::new(0), total: AtomicU64::new(0), cancel: AtomicBool::new(false), finished: AtomicBool::new(false), detail: Mutex::new(String::new()), result: Mutex::new(None) });
        self.list.push(job.clone());
        std::thread::spawn(move || {
            // Une panique ne doit pas laisser la tâche « en cours » indéfiniment.
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(&job))).unwrap_or_else(|_| Err("erreur interne".into()));
            if let Ok(mut g) = job.result.lock() {
                *g = Some(r);
            }
            job.finished.store(true, Ordering::SeqCst);
        });
        true
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
        out
    }
}
