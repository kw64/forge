//! Instance unique : les fichiers ouverts depuis l'explorateur (sélection multiple) sont transmis
//! à la fenêtre déjà ouverte via une socket locale (127.0.0.1). Un port par application. v0.4.0
use std::io::{BufRead, BufReader, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

const HELLO: &str = "MEDIAFORGE1";

pub enum Acquired {
    /// Les fichiers ont été transmis à l'instance existante : on quitte.
    Forwarded,
    /// Cette instance est la principale (écouteur éventuel).
    Primary(Option<TcpListener>),
}

fn try_forward(port: u16, files: &[PathBuf]) -> bool {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let Ok(mut s) = TcpStream::connect_timeout(&addr, Duration::from_millis(300)) else { return false };
    let _ = s.set_read_timeout(Some(Duration::from_secs(6)));
    if writeln!(s, "{HELLO}").is_err() {
        return false;
    }
    let mut reply = String::new();
    if BufReader::new(&s).read_line(&mut reply).is_err() || reply.trim() != "OK" {
        return false;
    }
    for f in files {
        let _ = writeln!(s, "{}", f.display());
    }
    let _ = s.shutdown(Shutdown::Write);
    true
}

/// Détermine le rôle de ce processus. Avec des fichiers à ouvrir, on retente quelques instants
/// (plusieurs processus peuvent démarrer ensemble lors d'une sélection multiple).
pub fn acquire(port: u16, files: &[PathBuf]) -> Acquired {
    let tries = if files.is_empty() { 1 } else { 25 };
    for _ in 0..tries {
        if !files.is_empty() && try_forward(port, files) {
            return Acquired::Forwarded;
        }
        if let Ok(l) = TcpListener::bind(("127.0.0.1", port)) {
            return Acquired::Primary(Some(l));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Acquired::Primary(None)
}

/// Démarre l'écoute ; les lots de fichiers reçus arrivent sur le `Receiver`.
pub fn serve(listener: TcpListener, on_batch: impl Fn() + Send + 'static) -> Receiver<Vec<PathBuf>> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        for conn in listener.incoming().flatten() {
            let _ = conn.set_read_timeout(Some(Duration::from_secs(5)));
            let mut w = match conn.try_clone() {
                Ok(w) => w,
                Err(_) => continue,
            };
            let mut lines = BufReader::new(conn).lines().map_while(Result::ok);
            if lines.next().as_deref() != Some(HELLO) {
                continue;
            }
            let _ = writeln!(w, "OK");
            let batch: Vec<PathBuf> = lines.filter(|l| !l.is_empty()).map(PathBuf::from).collect();
            if !batch.is_empty() {
                let _ = tx.send(batch);
                on_batch();
            }
        }
    });
    rx
}
