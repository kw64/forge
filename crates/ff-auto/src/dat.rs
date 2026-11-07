//! Identification des ROMs via fichiers DAT (No-Intro, TOSEC, Redump : format Logiqx XML) par CRC32. v0.1.0
use std::collections::HashMap;
use std::path::Path;

#[derive(Clone, Debug)]
pub struct Entry {
    pub game: String,
    pub rom: String,
    pub size: u64,
}

#[derive(Default)]
pub struct DatIndex {
    by_crc: HashMap<u32, Vec<Entry>>,
    pub games: Vec<String>,
    pub sets: Vec<String>,
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let key = format!("{name}=\"");
    let mut from = 0;
    while let Some(i) = tag[from..].find(&key) {
        let at = from + i;
        // l'attribut doit être précédé d'un espace (évite « name » dans « rename »)
        if at == 0 || tag.as_bytes()[at - 1].is_ascii_whitespace() {
            let s = at + key.len();
            let e = tag[s..].find('"')?;
            return Some(unescape(&tag[s..s + e]));
        }
        from = at + key.len();
    }
    None
}

fn unescape(s: &str) -> String {
    s.replace("&amp;", "&").replace("&apos;", "'").replace("&quot;", "\"").replace("&lt;", "<").replace("&gt;", ">")
}

impl DatIndex {
    pub fn parse_str(&mut self, xml: &str) {
        let mut game = String::new();
        let mut rest = xml;
        let mut added_set = false;
        while let Some(i) = rest.find('<') {
            rest = &rest[i + 1..];
            let Some(e) = rest.find('>') else { break };
            let tag = &rest[..e];
            if tag.starts_with("game ") || tag.starts_with("machine ") {
                game = attr(tag, "name").unwrap_or_default();
                self.games.push(game.clone());
                added_set = true;
            } else if tag.starts_with("rom ") {
                if let (Some(crc), Some(rom)) = (attr(tag, "crc"), attr(tag, "name")) {
                    if let Ok(c) = u32::from_str_radix(crc.trim(), 16) {
                        let size = attr(tag, "size").and_then(|s| s.parse().ok()).unwrap_or(0);
                        self.by_crc.entry(c).or_default().push(Entry { game: game.clone(), rom, size });
                    }
                }
            }
            rest = &rest[e..];
        }
        let _ = added_set;
    }

    pub fn load(&mut self, p: &Path) -> Result<usize, String> {
        let xml = std::fs::read_to_string(p).or_else(|_| std::fs::read(p).map(|b| String::from_utf8_lossy(&b).to_string())).map_err(|e| format!("{} : {e}", p.display()))?;
        let before = self.games.len();
        self.parse_str(&xml);
        self.sets.push(p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
        Ok(self.games.len() - before)
    }

    pub fn lookup(&self, crc: u32, size: u64) -> Option<&Entry> {
        let v = self.by_crc.get(&crc)?;
        v.iter().find(|e| e.size == size || e.size == 0).or(v.first())
    }

    pub fn is_empty(&self) -> bool {
        self.games.is_empty()
    }
}

pub fn crc32_file(p: &Path) -> Option<u32> {
    use std::io::Read;
    let mut f = std::fs::File::open(p).ok()?;
    let mut h = crc32fast::Hasher::new();
    let mut b = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut b).ok()?;
        if n == 0 { break; }
        h.update(&b[..n]);
    }
    Some(h.finalize())
}

/// CRC32 + taille de la ROM : fichier brut, ou entrée unique d'un ZIP (CRC lu dans l'annuaire, sans décompresser).
pub fn rom_crc(p: &Path) -> Option<(u32, u64)> {
    if p.extension().map(|e| e.eq_ignore_ascii_case("zip")).unwrap_or(false) {
        let mut z = zip::ZipArchive::new(std::fs::File::open(p).ok()?).ok()?;
        if z.len() == 1 {
            let e = z.by_index_raw(0).ok()?;
            return Some((e.crc32(), e.size()));
        }
        return None;
    }
    Some((crc32_file(p)?, std::fs::metadata(p).ok()?.len()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dat_lookup() {
        let mut d = DatIndex::default();
        d.parse_str(r#"<datafile><game name="Super Mario Bros. (World)"><description>x</description><rom name="Super Mario Bros. (World).nes" size="40976" crc="3337EC46" md5="a" sha1="b"/></game><game name="Tom &amp; Jerry"><rom name="tj.nes" size="10" crc="0000000A"/></game></datafile>"#);
        assert_eq!(d.lookup(0x3337EC46, 40976).unwrap().game, "Super Mario Bros. (World)");
        assert_eq!(d.lookup(10, 10).unwrap().game, "Tom & Jerry");
        assert!(d.lookup(1, 1).is_none());
        assert_eq!(d.games.len(), 2);
    }
}
