//! Liste de lecture : ordre normal ou aléatoire, répétition. Logique pure (testable). v0.4.0
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Repeat {
    #[default]
    Off,
    All,
    One,
}

impl Repeat {
    pub fn cycle(self) -> Self {
        match self {
            Repeat::Off => Repeat::All,
            Repeat::All => Repeat::One,
            Repeat::One => Repeat::Off,
        }
    }
}

#[derive(Default)]
pub struct Playlist {
    pub items: Vec<PathBuf>,
    pub cur: Option<usize>,
    pub shuffle: bool,
    pub repeat: Repeat,
    /// Ordre de lecture en mode aléatoire (indices dans `items`).
    order: Vec<usize>,
    seed: u64,
}

impl Playlist {
    fn rand(&mut self) -> u64 {
        if self.seed == 0 {
            self.seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(88172645463325252) | 1;
        }
        // xorshift64
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 7;
        self.seed ^= self.seed << 17;
        self.seed
    }

    fn rebuild_order(&mut self) {
        let n = self.items.len();
        let mut o: Vec<usize> = (0..n).collect();
        for i in (1..n).rev() {
            let j = (self.rand() % (i as u64 + 1)) as usize;
            o.swap(i, j);
        }
        // le morceau courant passe en tête
        if let Some(c) = self.cur {
            if let Some(p) = o.iter().position(|&x| x == c) {
                o.swap(0, p);
            }
        }
        self.order = o;
    }

    /// Remplace la liste et se positionne sur `start`.
    pub fn set(&mut self, items: Vec<PathBuf>, start: usize) {
        self.cur = if items.is_empty() { None } else { Some(start.min(items.len() - 1)) };
        self.items = items;
        self.rebuild_order();
    }

    pub fn append(&mut self, new: Vec<PathBuf>) {
        for p in new {
            if !self.items.contains(&p) {
                self.items.push(p);
            }
        }
        self.rebuild_order();
    }

    pub fn remove(&mut self, i: usize) {
        if i >= self.items.len() {
            return;
        }
        self.items.remove(i);
        self.cur = match self.cur {
            Some(c) if c == i => None,
            Some(c) if c > i => Some(c - 1),
            other => other,
        };
        self.rebuild_order();
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.cur = None;
        self.order.clear();
    }

    pub fn set_shuffle(&mut self, on: bool) {
        self.shuffle = on;
        if on {
            self.rebuild_order();
        }
    }

    pub fn current(&self) -> Option<&PathBuf> {
        self.cur.and_then(|c| self.items.get(c))
    }

    /// Indice du morceau suivant. `auto` = fin naturelle d'un fichier (la répétition d'un seul fichier s'applique).
    pub fn next_index(&self, auto: bool) -> Option<usize> {
        let (n, cur) = (self.items.len(), self.cur?);
        if n == 0 {
            return None;
        }
        if auto && self.repeat == Repeat::One {
            return Some(cur);
        }
        if self.shuffle {
            let p = self.order.iter().position(|&x| x == cur)?;
            return match self.order.get(p + 1) {
                Some(&i) => Some(i),
                None if self.repeat == Repeat::All || !auto => self.order.first().copied(),
                None => None,
            };
        }
        if cur + 1 < n {
            Some(cur + 1)
        } else if self.repeat == Repeat::All || !auto {
            Some(0)
        } else {
            None
        }
    }

    pub fn prev_index(&self) -> Option<usize> {
        let (n, cur) = (self.items.len(), self.cur?);
        if n == 0 {
            return None;
        }
        if self.shuffle {
            let p = self.order.iter().position(|&x| x == cur)?;
            return Some(if p == 0 { *self.order.last()? } else { self.order[p - 1] });
        }
        Some(if cur == 0 { n - 1 } else { cur - 1 })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pl(n: usize) -> Playlist {
        let mut p = Playlist::default();
        p.set((0..n).map(|i| PathBuf::from(format!("f{i}.mp3"))).collect(), 0);
        p
    }

    #[test]
    fn sequential_and_repeat() {
        let mut p = pl(3);
        assert_eq!(p.next_index(true), Some(1));
        p.cur = Some(2);
        assert_eq!(p.next_index(true), None); // fin de liste, pas de répétition
        assert_eq!(p.next_index(false), Some(0)); // « suivant » manuel boucle
        p.repeat = Repeat::All;
        assert_eq!(p.next_index(true), Some(0));
        p.repeat = Repeat::One;
        assert_eq!(p.next_index(true), Some(2));
        assert_eq!(p.next_index(false), Some(0));
        assert_eq!(p.prev_index(), Some(1));
    }

    #[test]
    fn shuffle_visits_everything_once() {
        let mut p = pl(8);
        p.cur = Some(3);
        p.set_shuffle(true);
        let mut seen = vec![p.cur.unwrap()];
        while let Some(n) = p.next_index(true) {
            p.cur = Some(n);
            seen.push(n);
        }
        seen.sort();
        assert_eq!(seen, (0..8).collect::<Vec<_>>());
    }

    #[test]
    fn remove_keeps_current() {
        let mut p = pl(4);
        p.cur = Some(2);
        p.remove(0);
        assert_eq!(p.cur, Some(1));
        assert_eq!(p.current().unwrap().to_string_lossy(), "f2.mp3");
        p.remove(1);
        assert_eq!(p.cur, None);
    }

    #[test]
    fn append_skips_duplicates() {
        let mut p = pl(2);
        p.append(vec![PathBuf::from("f1.mp3"), PathBuf::from("new.mp3")]);
        assert_eq!(p.items.len(), 3);
    }
}
