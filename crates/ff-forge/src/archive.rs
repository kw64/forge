//! Archives packées : lecture/navigation/extraction (ZIP, 7z, TAR, TAR.GZ, LHA/LZH, RAR via 7-Zip) et création (ZIP, 7z, TAR.GZ). v0.6.0
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, BufReader, Read, Write as _};
use std::path::{Path, PathBuf};
use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::sync::Mutex;

/// Codes d'erreur « mot de passe » : l'interface ouvre alors la demande de mot de passe.
pub const NEED_PW: &str = "PASSWORD_REQUIRED";
pub const BAD_PW: &str = "PASSWORD_BAD";

static PASSWORDS: Mutex<Option<HashMap<PathBuf, String>>> = Mutex::new(None);
static PW_REQUEST: Mutex<Option<(PathBuf, bool)>> = Mutex::new(None);

/// Mémorise le mot de passe d'une archive pour la session.
pub fn set_password(p: &Path, pw: &str) {
    if let Ok(mut g) = PASSWORDS.lock() {
        g.get_or_insert_with(HashMap::new).insert(p.to_path_buf(), pw.to_string());
    }
}

fn password(p: &Path) -> Option<String> {
    PASSWORDS.lock().ok()?.as_ref()?.get(p).cloned()
}

fn need_pw(p: &Path, bad: bool) -> String {
    if let Ok(mut g) = PW_REQUEST.lock() {
        *g = Some((p.to_path_buf(), bad));
    }
    if bad { BAD_PW } else { NEED_PW }.to_string()
}

/// Demande de mot de passe en attente (archive, mot de passe précédent refusé ?).
pub fn take_pw_request() -> Option<(PathBuf, bool)> {
    PW_REQUEST.lock().ok()?.take()
}

pub fn is_pw_error(e: &str) -> bool {
    e == NEED_PW || e == BAD_PW
}

/// Exécutable RAR (WinRAR) pour créer des .rar.
pub fn rar_exe() -> Option<PathBuf> {
    let mut c: Vec<PathBuf> = vec!["rar".into()];
    if cfg!(windows) {
        for b in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(d) = std::env::var_os(b) {
                c.push(Path::new(&d).join("WinRAR").join("Rar.exe"));
            }
        }
    }
    c.into_iter().find(|p| Command::new(p).stdin(Stdio::null()).output().is_ok())
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Zip,
    SevenZ,
    Tar,
    TarGz,
    Lha,
    /// RAR et autres : délégué à 7-Zip s'il est installé.
    External,
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub size: u64,
    pub is_dir: bool,
}

pub struct Archive {
    pub path: PathBuf,
    pub kind: Kind,
    pub entries: Vec<Entry>,
}

/// Type d'archive d'après l'extension.
pub fn kind_of(p: &Path) -> Option<Kind> {
    let n = p.file_name()?.to_str()?.to_ascii_lowercase();
    let ext = |e: &str| n.ends_with(e);
    Some(if ext(".tar.gz") || ext(".tgz") {
        Kind::TarGz
    } else if ext(".gz") {
        Kind::External
    } else if ext(".tar") {
        Kind::Tar
    } else if ext(".zip") || ext(".jar") || ext(".cbz") || ext(".apk") {
        Kind::Zip
    } else if ext(".7z") {
        Kind::SevenZ
    } else if ext(".lha") || ext(".lzh") || ext(".lzs") {
        Kind::Lha
    } else if ext(".rar") || ext(".cab") || ext(".iso") || ext(".xz") || ext(".bz2") || ext(".cbr") {
        Kind::External
    } else {
        return None;
    })
}

fn norm(n: &str) -> String {
    n.replace('\\', "/").trim_start_matches("./").trim_matches('/').to_string()
}

/// Chemin de destination sûr (pas de `..`, pas de racine).
fn safe_join(dest: &Path, rel: &str) -> PathBuf {
    let mut p = dest.to_path_buf();
    for c in rel.split('/') {
        if !c.is_empty() && c != "." && c != ".." && !c.contains(':') {
            p.push(c);
        }
    }
    p
}

fn write_entry(dest: &Path, rel: &str, is_dir: bool, r: &mut dyn Read) -> Result<(), String> {
    let out = safe_join(dest, rel);
    if is_dir {
        return std::fs::create_dir_all(&out).map_err(|e| e.to_string());
    }
    if let Some(d) = out.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let mut f = File::create(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    io::copy(r, &mut f).map_err(|e| e.to_string())?;
    Ok(())
}

fn seven_zip_exe() -> Option<PathBuf> {
    let mut c: Vec<PathBuf> = vec!["7z".into(), "7zz".into(), "7za".into()];
    if cfg!(windows) {
        for b in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(d) = std::env::var_os(b) {
                c.push(Path::new(&d).join("7-Zip").join("7z.exe"));
            }
        }
    }
    c.into_iter().find(|p| Command::new(p).arg("-h").output().map(|o| o.status.success()).unwrap_or(false))
}

impl Archive {
    pub fn open(path: &Path) -> Result<Archive, String> {
        let kind = kind_of(path).ok_or("format d'archive non reconnu")?;
        let mut entries = Vec::new();
        match kind {
            Kind::Zip => {
                let mut z = zip::ZipArchive::new(BufReader::new(File::open(path).map_err(|e| e.to_string())?)).map_err(|e| e.to_string())?;
                for i in 0..z.len() {
                    // « raw » : les noms restent lisibles même si le contenu est chiffré.
                    if let Ok(f) = z.by_index_raw(i) {
                        entries.push(Entry { name: norm(f.name()), size: f.size(), is_dir: f.is_dir() });
                    }
                }
            }
            Kind::Tar | Kind::TarGz => {
                let mut a = tar_open(path, kind)?;
                for e in a.entries().map_err(|e| e.to_string())?.flatten() {
                    let n = e.path().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
                    entries.push(Entry { name: norm(&n), size: e.size(), is_dir: e.header().entry_type().is_dir() });
                }
            }
            Kind::SevenZ => {
                let pw = password(path);
                let a = sevenz_rust::Archive::open_with_password(path, &pw.as_deref().map(sevenz_rust::Password::from).unwrap_or_else(sevenz_rust::Password::empty)).map_err(|e| sz_err(path, e, pw.is_some()))?;
                for f in &a.files {
                    entries.push(Entry { name: norm(&f.name), size: f.size, is_dir: f.is_directory });
                }
            }
            Kind::Lha => {
                let mut r = delharc::parse_file(path).map_err(|e| e.to_string())?;
                loop {
                    let h = r.header();
                    entries.push(Entry { name: norm(&h.parse_pathname_to_str()), size: h.original_size, is_dir: h.is_directory() });
                    if !r.next_file().map_err(|e| e.to_string())? {
                        break;
                    }
                }
            }
            Kind::External => {
                let exe = seven_zip_exe().ok_or("7-Zip est requis pour ce format (installez 7-Zip)")?;
                let mut cmd = Command::new(exe);
                cmd.args(["l", "-slt", "-ba"]);
                if let Some(pw) = password(path) {
                    cmd.arg(format!("-p{pw}"));
                }
                let out = cmd.arg(path).stdin(Stdio::null()).output().map_err(|e| e.to_string())?;
                let txt0 = String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
                if !out.status.success() && (txt0.contains("Wrong password") || txt0.contains("Enter password") || txt0.contains("encrypted")) {
                    return Err(need_pw(path, password(path).is_some()));
                }
                let txt = String::from_utf8_lossy(&out.stdout).to_string();
                for blk in txt.split("\n\n") {
                    let (mut name, mut size, mut dir) = (None, 0u64, false);
                    for l in blk.lines() {
                        if let Some(v) = l.strip_prefix("Path = ") {
                            name = Some(norm(v));
                        } else if let Some(v) = l.strip_prefix("Size = ") {
                            size = v.trim().parse().unwrap_or(0);
                        } else if let Some(v) = l.strip_prefix("Folder = ") {
                            dir = v.trim() == "+";
                        }
                    }
                    if let Some(n) = name {
                        entries.push(Entry { name: n, size, is_dir: dir });
                    }
                }
            }
        }
        entries.retain(|e| !e.name.is_empty());
        Ok(Archive { path: path.to_path_buf(), kind, entries })
    }

    /// Contenu immédiat du dossier `inner` (les dossiers implicites sont synthétisés).
    pub fn list(&self, inner: &str) -> Vec<Entry> {
        let mut m: BTreeMap<String, Entry> = BTreeMap::new();
        for e in &self.entries {
            let rest = if inner.is_empty() {
                e.name.as_str()
            } else if let Some(r) = e.name.strip_prefix(inner).and_then(|r| r.strip_prefix('/')) {
                r
            } else {
                continue;
            };
            if rest.is_empty() {
                continue;
            }
            match rest.split_once('/') {
                Some((d, _)) => {
                    let full = if inner.is_empty() { d.to_string() } else { format!("{inner}/{d}") };
                    m.entry(full.clone()).or_insert(Entry { name: full, size: 0, is_dir: true });
                }
                None => {
                    m.insert(e.name.clone(), e.clone());
                }
            }
        }
        m.into_values().collect()
    }

    /// Extrait les entrées pour lesquelles `pred(nom)` est vrai ; `strip` est le préfixe retiré des chemins.
    pub fn extract(&self, pred: &dyn Fn(&str) -> bool, dest: &Path, strip: &str) -> Result<usize, String> {
        self.extract_p(pred, dest, strip, &|_| true)
    }

    /// Octets à extraire pour `pred` (pour la barre de progression).
    pub fn size_of(&self, pred: &dyn Fn(&str) -> bool) -> u64 {
        self.entries.iter().filter(|e| pred(&e.name)).map(|e| e.size).sum()
    }

    /// Extraction avec progression : `prog(octets)` est appelé après chaque entrée ; `false` annule.
    pub fn extract_p(&self, pred: &dyn Fn(&str) -> bool, dest: &Path, strip: &str, prog: &dyn Fn(u64) -> bool) -> Result<usize, String> {
        let rel = |n: &str| -> String {
            if strip.is_empty() { n.to_string() } else { n.strip_prefix(strip).map(|r| r.trim_start_matches('/').to_string()).unwrap_or_else(|| n.to_string()) }
        };
        let mut count = 0usize;
        std::fs::create_dir_all(dest).map_err(|e| e.to_string())?;
        match self.kind {
            Kind::Zip => {
                let mut z = zip::ZipArchive::new(BufReader::new(File::open(&self.path).map_err(|e| e.to_string())?)).map_err(|e| e.to_string())?;
                for i in 0..z.len() {
                    let (n, enc) = {
                        let f = z.by_index_raw(i).map_err(|e| e.to_string())?;
                        (norm(f.name()), f.encrypted())
                    };
                    if !pred(&n) {
                        continue;
                    }
                    let mut f = if enc {
                        let Some(pw) = password(&self.path) else { return Err(need_pw(&self.path, false)) };
                        z.by_index_decrypt(i, pw.as_bytes()).map_err(|e| match e {
                            zip::result::ZipError::InvalidPassword => need_pw(&self.path, true),
                            e => e.to_string(),
                        })?
                    } else {
                        z.by_index(i).map_err(|e| e.to_string())?
                    };
                    let d = f.is_dir();
                    let sz = f.size();
                    write_entry(dest, &rel(&n), d, &mut f)?;
                    count += 1;
                    if !prog(sz) {
                        return Err("Annulé".into());
                    }
                }
            }
            Kind::Tar | Kind::TarGz => {
                let mut a = tar_open(&self.path, self.kind)?;
                for e in a.entries().map_err(|e| e.to_string())? {
                    let mut e = e.map_err(|e| e.to_string())?;
                    let n = norm(&e.path().map_err(|e| e.to_string())?.to_string_lossy());
                    if pred(&n) {
                        let d = e.header().entry_type().is_dir();
                        let sz = e.size();
                        write_entry(dest, &rel(&n), d, &mut e)?;
                        count += 1;
                        if !prog(sz) {
                            return Err("Annulé".into());
                        }
                    }
                }
            }
            Kind::SevenZ => {
                let pw = password(&self.path);
                let mut r = sevenz_rust::SevenZReader::open(&self.path, pw.as_deref().map(sevenz_rust::Password::from).unwrap_or_else(sevenz_rust::Password::empty)).map_err(|e| sz_err(&self.path, e, pw.is_some()))?;
                let mut err: Option<String> = None;
                r.for_each_entries(|e, rd| {
                    let n = norm(&e.name);
                    if pred(&n) {
                        if let Err(x) = write_entry(dest, &rel(&n), e.is_directory, rd) {
                            err = Some(x);
                            return Ok(false);
                        }
                        count += 1;
                        if !prog(e.size) {
                            err = Some("Annulé".into());
                            return Ok(false);
                        }
                    }
                    Ok(true)
                })
                .map_err(|e| sz_err(&self.path, e, pw.is_some()))?;
                if let Some(e) = err {
                    return Err(e);
                }
            }
            Kind::Lha => {
                let mut r = delharc::parse_file(&self.path).map_err(|e| e.to_string())?;
                loop {
                    let h = r.header();
                    let n = norm(&h.parse_pathname_to_str());
                    let isd = h.is_directory();
                    let h_size = h.original_size;
                    if pred(&n) {
                        if isd {
                            write_entry(dest, &rel(&n), true, &mut io::empty())?;
                        } else if r.is_decoder_supported() {
                            write_entry(dest, &rel(&n), false, &mut r)?;
                        } else {
                            return Err(format!("méthode de compression non gérée : {n}"));
                        }
                        count += 1;
                        if !prog(h_size) {
                            return Err("Annulé".into());
                        }
                    }
                    if !r.next_file().map_err(|e| e.to_string())? {
                        break;
                    }
                }
            }
            Kind::External => {
                let exe = seven_zip_exe().ok_or("7-Zip est requis pour ce format")?;
                let tmp = dest.join(".ff_extract_tmp");
                let mut cmd = Command::new(exe);
                cmd.arg("x").arg("-y").arg(format!("-o{}", tmp.display()));
                if let Some(pw) = password(&self.path) {
                    cmd.arg(format!("-p{pw}"));
                }
                let out = cmd.arg(&self.path).stdin(Stdio::null()).output().map_err(|e| e.to_string())?;
                if !out.status.success() {
                    let _ = std::fs::remove_dir_all(&tmp);
                    let all = String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
                    if all.contains("Wrong password") || all.contains("Enter password") || all.contains("Can not open encrypted") {
                        return Err(need_pw(&self.path, password(&self.path).is_some()));
                    }
                    return Err(String::from_utf8_lossy(&out.stdout).lines().last().unwrap_or("échec 7-Zip").to_string());
                }
                for e in &self.entries {
                    if pred(&e.name) {
                        let src = safe_join(&tmp, &e.name);
                        let dst = safe_join(dest, &rel(&e.name));
                        if e.is_dir {
                            let _ = std::fs::create_dir_all(&dst);
                        } else {
                            if let Some(d) = dst.parent() {
                                let _ = std::fs::create_dir_all(d);
                            }
                            std::fs::copy(&src, &dst).map_err(|x| x.to_string())?;
                        }
                        count += 1;
                        if !prog(e.size) {
                            let _ = std::fs::remove_dir_all(&tmp);
                            return Err("Annulé".into());
                        }
                    }
                }
                let _ = std::fs::remove_dir_all(&tmp);
            }
        }
        Ok(count)
    }
}

fn tar_open(path: &Path, kind: Kind) -> Result<tar::Archive<Box<dyn Read>>, String> {
    let f = BufReader::new(File::open(path).map_err(|e| e.to_string())?);
    let r: Box<dyn Read> = if kind == Kind::TarGz { Box::new(flate2::read::GzDecoder::new(f)) } else { Box::new(f) };
    Ok(tar::Archive::new(r))
}

/// Erreur 7z → message, ou demande de mot de passe.
fn sz_err(path: &Path, e: sevenz_rust::Error, had_pw: bool) -> String {
    match e {
        sevenz_rust::Error::PasswordRequired => need_pw(path, had_pw),
        sevenz_rust::Error::MaybeBadPassword(_) => need_pw(path, true),
        e => e.to_string(),
    }
}

/// Lecteur qui signale la progression (et permet l'annulation en cours de fichier).
struct ProgressRead<'a, R> {
    r: R,
    prog: &'a dyn Fn(u64) -> bool,
}

impl<R: Read> Read for ProgressRead<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.r.read(buf)?;
        if n > 0 && !(self.prog)(n as u64) {
            return Err(io::Error::other("Annulé"));
        }
        Ok(n)
    }
}

/// Ajoute récursivement `p` (fichier ou dossier) sous le nom `name` dans l'archive ZIP (AES-256 si mot de passe).
fn add_zip(w: &mut zip::ZipWriter<File>, p: &Path, name: &str, pw: Option<&str>, prog: &dyn Fn(u64) -> bool) -> Result<(), String> {
    let mut opt = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    if let Some(pw) = pw {
        opt = opt.with_aes_encryption(zip::AesMode::Aes256, pw);
    }
    if p.is_dir() {
        w.add_directory(format!("{name}/"), opt).map_err(|e| e.to_string())?;
        for e in std::fs::read_dir(p).map_err(|e| e.to_string())?.flatten() {
            add_zip(w, &e.path(), &format!("{name}/{}", e.file_name().to_string_lossy()), pw, prog)?;
        }
    } else {
        w.start_file(name, opt).map_err(|e| e.to_string())?;
        let mut r = ProgressRead { r: File::open(p).map_err(|e| e.to_string())?, prog };
        io::copy(&mut r, w).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Formats de création : (libellé, extension, chiffrement possible).
pub const FORMATS: &[(&str, &str, bool)] = &[("ZIP", ".zip", true), ("7Z", ".7z", true), ("RAR", ".rar", true), ("TAR.GZ", ".tar.gz", false), ("TAR", ".tar", false), ("GZ", ".gz", false)];

/// Remplace l'extension d'archive connue de `name` par `ext`.
pub fn with_format_ext(name: &str, ext: &str) -> String {
    let low = name.to_ascii_lowercase();
    for e in [".tar.gz", ".tgz", ".zip", ".7z", ".rar", ".tar", ".gz"] {
        if low.ends_with(e) {
            return format!("{}{ext}", &name[..name.len() - e.len()]);
        }
    }
    format!("{name}{ext}")
}

/// Crée une archive d'après l'extension de `dest` (`zip`, `7z`, `rar`, `tar.gz`, `tar`, `gz`).
/// `password` : ZIP/7z (AES-256) et RAR (WinRAR requis) ; ignoré pour TAR/GZ.
#[allow(dead_code)]
pub fn create(dest: &Path, files: &[PathBuf], password: Option<&str>) -> Result<(), String> {
    create_p(dest, files, password, &|_| true)
}

/// Comme `create`, avec progression en octets (`false` = annuler ; l'archive partielle est supprimée).
pub fn create_p(dest: &Path, files: &[PathBuf], password: Option<&str>, prog: &dyn Fn(u64) -> bool) -> Result<(), String> {
    let r = create_inner(dest, files, password, prog);
    if r.is_err() {
        let _ = std::fs::remove_file(dest);
    }
    r
}

fn create_inner(dest: &Path, files: &[PathBuf], password: Option<&str>, prog: &dyn Fn(u64) -> bool) -> Result<(), String> {
    let n = dest.file_name().and_then(|n| n.to_str()).unwrap_or("").to_ascii_lowercase();
    let nm = |p: &Path| p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "fichier".into());
    let pw = password.filter(|p| !p.is_empty());
    if n.ends_with(".zip") {
        let mut w = zip::ZipWriter::new(File::create(dest).map_err(|e| e.to_string())?);
        for p in files {
            add_zip(&mut w, p, &nm(p), pw, prog)?;
        }
        w.finish().map_err(|e| e.to_string())?;
    } else if n.ends_with(".tar.gz") || n.ends_with(".tgz") || n.ends_with(".tar") {
        let out = File::create(dest).map_err(|e| e.to_string())?;
        let w: Box<dyn io::Write> = if n.ends_with(".tar") { Box::new(out) } else { Box::new(flate2::write::GzEncoder::new(out, flate2::Compression::default())) };
        let mut t = tar::Builder::new(w);
        for p in files {
            if p.is_dir() {
                t.append_dir_all(nm(p), p).map_err(|e| e.to_string())?;
            } else {
                t.append_path_with_name(p, nm(p)).map_err(|e| e.to_string())?;
            }
            if !prog(crate::fsops::tree_size(p)) {
                return Err("Annulé".into());
            }
        }
        t.into_inner().map_err(|e| e.to_string())?.flush().map_err(|e| e.to_string())?;
    } else if n.ends_with(".gz") {
        // GZ ne compresse qu'un seul fichier (utilisez TAR.GZ pour plusieurs fichiers ou un dossier).
        if files.len() != 1 || files[0].is_dir() {
            return Err("GZ : un seul fichier (utilisez TAR.GZ pour un dossier ou plusieurs fichiers)".into());
        }
        let mut gz = flate2::write::GzEncoder::new(File::create(dest).map_err(|e| e.to_string())?, flate2::Compression::default());
        let mut r = ProgressRead { r: File::open(&files[0]).map_err(|e| e.to_string())?, prog };
        io::copy(&mut r, &mut gz).map_err(|e| e.to_string())?;
        gz.finish().map_err(|e| e.to_string())?;
    } else if n.ends_with(".7z") || n.ends_with(".rar") {
        // Ces formats travaillent sur un dossier : on assemble la sélection dans un dossier temporaire.
        let tmp = std::env::temp_dir().join(format!("fileforge-pack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
        for p in files {
            crate::fsops::copy_recursive(p, &tmp.join(nm(p)))?;
            if !prog(crate::fsops::tree_size(p)) {
                let _ = std::fs::remove_dir_all(&tmp);
                return Err("Annulé".into());
            }
        }
        let r = if n.ends_with(".7z") {
            match pw {
                Some(pw) => sevenz_rust::compress_to_path_encrypted(&tmp, dest, sevenz_rust::Password::from(pw)).map_err(|e| e.to_string()),
                None => sevenz_rust::compress_to_path(&tmp, dest).map_err(|e| e.to_string()),
            }
        } else {
            let rar = rar_exe().ok_or("RAR : WinRAR (rar.exe) n'est pas installé — la création de RAR est impossible sans lui")?;
            let _ = std::fs::remove_file(dest);
            let mut c = Command::new(rar);
            c.current_dir(&tmp).args(["a", "-r", "-idq", "-y"]);
            if let Some(pw) = pw {
                c.arg(format!("-hp{pw}"));
            }
            c.arg(dest).arg("*").stdin(Stdio::null());
            match c.output() {
                Ok(o) if o.status.success() => Ok(()),
                Ok(o) => Err(String::from_utf8_lossy(&o.stdout).lines().chain(String::from_utf8_lossy(&o.stderr).lines()).last().unwrap_or("échec de RAR").to_string()),
                Err(e) => Err(e.to_string()),
            }
        };
        let _ = std::fs::remove_dir_all(&tmp);
        r?;
    } else {
        return Err("extension d'archive inconnue".into());
    }
    Ok(())
}

#[cfg(test)]
mod pw_tests {
    use super::*;

    fn roundtrip(ext: &str) {
        let d = std::env::temp_dir().join(format!("ffpw{}{}", std::process::id(), ext.replace('.', "_")));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src/sub")).unwrap();
        std::fs::write(d.join("src/a.txt"), "secret A").unwrap();
        std::fs::write(d.join("src/sub/b.txt"), "secret B").unwrap();
        let arc = d.join(format!("t{ext}"));
        create(&arc, &[d.join("src")], Some("s3cr3t")).unwrap();
        // Sans mot de passe : demandé.
        let e = Archive::open(&arc).and_then(|a| a.extract(&|_| true, &d.join("o1"), "")).unwrap_err();
        assert_eq!(e, NEED_PW, "{ext}");
        assert!(take_pw_request().is_some());
        // Mauvais mot de passe.
        set_password(&arc, "nope");
        let e = Archive::open(&arc).and_then(|a| a.extract(&|_| true, &d.join("o2"), "")).unwrap_err();
        assert!(e == BAD_PW || e == NEED_PW, "{ext}: {e}");
        // Bon mot de passe.
        set_password(&arc, "s3cr3t");
        let a = Archive::open(&arc).unwrap();
        a.extract(&|_| true, &d.join("o3"), "").unwrap();
        assert_eq!(std::fs::read_to_string(d.join("o3/src/sub/b.txt")).unwrap(), "secret B");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn zip_password() {
        roundtrip(".zip");
    }

    #[test]
    fn sevenz_password() {
        roundtrip(".7z");
    }

    #[test]
    fn tar_gz_and_gz() {
        let d = std::env::temp_dir().join(format!("ffgz{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("f.txt"), "hello").unwrap();
        create(&d.join("f.txt.gz"), &[d.join("f.txt")], None).unwrap();
        create(&d.join("x.tar"), &[d.join("f.txt")], None).unwrap();
        create(&d.join("x.tar.gz"), &[d.join("f.txt")], None).unwrap();
        for n in ["x.tar", "x.tar.gz"] {
            let a = Archive::open(&d.join(n)).unwrap();
            assert_eq!(a.entries.len(), 1);
        }
        assert_eq!(with_format_ext("doc.tar.gz", ".zip"), "doc.zip");
        assert_eq!(with_format_ext("dossier", ".7z"), "dossier.7z");
        let _ = std::fs::remove_dir_all(&d);
    }
}
