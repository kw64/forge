//! ff-mpv : liaison dynamique à libmpv (aucun fichier de développement requis à la compilation).
//! La bibliothèque (libmpv-2.dll, libmpv.so.2…) est chargée à l'exécution ; son absence est une erreur claire. v0.4.0
//! Licence : MediaForge lie libmpv dynamiquement (LGPL) ; ne distribuez que des builds libmpv LGPL pour rester en LGPL.
#![allow(clippy::missing_safety_doc)]

use libloading::Library;
use std::ffi::{c_char, c_double, c_int, c_void, CStr, CString};
use std::path::PathBuf;
use std::sync::Arc;

// ---------- types C ----------

#[repr(C)]
struct RawEvent {
    event_id: c_int,
    error: c_int,
    reply_userdata: u64,
    data: *mut c_void,
}
#[repr(C)]
struct RawEndFile {
    reason: c_int,
    error: c_int,
}
#[repr(C)]
struct RenderParam {
    kind: c_int,
    data: *mut c_void,
}
#[repr(C)]
struct OglInit {
    get_proc_address: Option<unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void>,
    ctx: *mut c_void,
}
#[repr(C)]
struct OglFbo {
    fbo: c_int,
    w: c_int,
    h: c_int,
    internal_format: c_int,
}

#[allow(dead_code)]
const FORMAT_DOUBLE: c_int = 5;
const R_API_TYPE: c_int = 1;
const R_OPENGL_INIT: c_int = 2;
const R_OPENGL_FBO: c_int = 3;
const R_FLIP_Y: c_int = 4;

type PCreate = unsafe extern "C" fn() -> *mut c_void;
type PInit = unsafe extern "C" fn(*mut c_void) -> c_int;
type PDestroy = unsafe extern "C" fn(*mut c_void);
type PSetStr = unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> c_int;
type PGetStr = unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_char;
type PCommand = unsafe extern "C" fn(*mut c_void, *mut *const c_char) -> c_int;
type PFree = unsafe extern "C" fn(*mut c_void);
type PErr = unsafe extern "C" fn(c_int) -> *const c_char;
type PWait = unsafe extern "C" fn(*mut c_void, c_double) -> *mut RawEvent;
type PRCreate = unsafe extern "C" fn(*mut *mut c_void, *mut c_void, *mut RenderParam) -> c_int;
type PRRender = unsafe extern "C" fn(*mut c_void, *mut RenderParam) -> c_int;
type PRFree = unsafe extern "C" fn(*mut c_void);
type PRCallback = unsafe extern "C" fn(*mut c_void, Option<unsafe extern "C" fn(*mut c_void)>, *mut c_void);
type PRUpdate = unsafe extern "C" fn(*mut c_void) -> u64;

/// Table des fonctions libmpv utilisées.
struct Api {
    _lib: Library,
    create: PCreate,
    initialize: PInit,
    terminate_destroy: PDestroy,
    set_option_string: PSetStr,
    set_property_string: PSetStr,
    get_property_string: PGetStr,
    command: PCommand,
    free: PFree,
    error_string: PErr,
    wait_event: PWait,
    render_create: PRCreate,
    render_render: PRRender,
    render_free: PRFree,
    render_set_callback: PRCallback,
    render_update: PRUpdate,
}

macro_rules! sym {
    ($lib:expr, $name:literal, $ty:ty) => {
        *unsafe { $lib.get::<$ty>(concat!($name, "\0").as_bytes()) }.map_err(|e| format!("symbole {} absent : {e}", $name))?
    };
}

/// Noms de la bibliothèque selon la plateforme.
fn candidates() -> Vec<PathBuf> {
    let names: &[&str] = if cfg!(windows) {
        &["libmpv-2.dll", "mpv-2.dll", "libmpv.dll", "mpv.dll"]
    } else if cfg!(target_os = "macos") {
        &["libmpv.2.dylib", "libmpv.dylib"]
    } else {
        &["libmpv.so.2", "libmpv.so.1", "libmpv.so"]
    };
    let mut v = Vec::new();
    if let Ok(dir) = std::env::var("MEDIAFORGE_MPV_DIR") {
        v.extend(names.iter().map(|n| PathBuf::from(&dir).join(n)));
    }
    if let Some(d) = std::env::var_os("APPDIR") {
        v.extend(names.iter().map(|n| PathBuf::from(&d).join("usr/lib").join(n)));
    }
    if let Some(exe_dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.to_path_buf())) {
        v.extend(names.iter().map(|n| exe_dir.join(n)));
        v.extend(names.iter().map(|n| exe_dir.join("mpv").join(n)));
    }
    v.extend(names.iter().map(PathBuf::from)); // recherche système (PATH, ld.so)
    v
}

fn load() -> Result<Api, String> {
    let mut last = String::from("aucun candidat");
    for c in candidates() {
        match unsafe { Library::new(&c) } {
            Ok(lib) => {
                return Ok(Api {
                    create: sym!(lib, "mpv_create", PCreate),
                    initialize: sym!(lib, "mpv_initialize", PInit),
                    terminate_destroy: sym!(lib, "mpv_terminate_destroy", PDestroy),
                    set_option_string: sym!(lib, "mpv_set_option_string", PSetStr),
                    set_property_string: sym!(lib, "mpv_set_property_string", PSetStr),
                    get_property_string: sym!(lib, "mpv_get_property_string", PGetStr),
                    command: sym!(lib, "mpv_command", PCommand),
                    free: sym!(lib, "mpv_free", PFree),
                    error_string: sym!(lib, "mpv_error_string", PErr),
                    wait_event: sym!(lib, "mpv_wait_event", PWait),
                    render_create: sym!(lib, "mpv_render_context_create", PRCreate),
                    render_render: sym!(lib, "mpv_render_context_render", PRRender),
                    render_free: sym!(lib, "mpv_render_context_free", PRFree),
                    render_set_callback: sym!(lib, "mpv_render_context_set_update_callback", PRCallback),
                    render_update: sym!(lib, "mpv_render_context_update", PRUpdate),
                    _lib: lib,
                });
            }
            Err(e) => last = format!("{}: {e}", c.display()),
        }
    }
    Err(format!("libmpv introuvable ({last})"))
}

// ---------- API sûre ----------

struct Inner {
    api: Api,
    handle: *mut c_void,
}
unsafe impl Send for Inner {}
unsafe impl Sync for Inner {}
impl Drop for Inner {
    fn drop(&mut self) {
        unsafe { (self.api.terminate_destroy)(self.handle) }
    }
}

/// Raison de fin de lecture d'un fichier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndReason {
    Eof,
    Stop,
    Quit,
    Error,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    None,
    Shutdown,
    StartFile,
    FileLoaded,
    EndFile(EndReason),
    VideoReconfig,
    PlaybackRestart,
    Other(i32),
}

/// Lecteur mpv (le handle libmpv est thread-safe).
#[derive(Clone)]
pub struct Mpv {
    inner: Arc<Inner>,
}

impl Mpv {
    /// Charge libmpv, applique les options (avant initialisation) puis initialise.
    pub fn new(options: &[(&str, &str)]) -> Result<Self, String> {
        let api = load()?;
        let handle = unsafe { (api.create)() };
        if handle.is_null() {
            return Err("mpv_create a échoué".into());
        }
        let inner = Arc::new(Inner { api, handle });
        let me = Mpv { inner };
        for (k, v) in options {
            // Une option inconnue d'une autre version de libmpv ne doit pas empêcher le démarrage.
            let _ = me.raw_set_option(k, v);
        }
        let rc = unsafe { (me.inner.api.initialize)(me.inner.handle) };
        if rc < 0 {
            return Err(format!("mpv_initialize : {}", me.err(rc)));
        }
        Ok(me)
    }

    fn err(&self, code: c_int) -> String {
        unsafe { CStr::from_ptr((self.inner.api.error_string)(code)).to_string_lossy().into_owned() }
    }

    fn raw_set_option(&self, k: &str, v: &str) -> Result<(), String> {
        let (k, v) = (CString::new(k).map_err(|e| e.to_string())?, CString::new(v).map_err(|e| e.to_string())?);
        let rc = unsafe { (self.inner.api.set_option_string)(self.inner.handle, k.as_ptr(), v.as_ptr()) };
        if rc < 0 { Err(self.err(rc)) } else { Ok(()) }
    }

    pub fn set_property(&self, name: &str, value: &str) -> Result<(), String> {
        let (k, v) = (CString::new(name).map_err(|e| e.to_string())?, CString::new(value).map_err(|e| e.to_string())?);
        let rc = unsafe { (self.inner.api.set_property_string)(self.inner.handle, k.as_ptr(), v.as_ptr()) };
        if rc < 0 { Err(format!("{name}: {}", self.err(rc))) } else { Ok(()) }
    }

    pub fn get_string(&self, name: &str) -> Option<String> {
        let k = CString::new(name).ok()?;
        let p = unsafe { (self.inner.api.get_property_string)(self.inner.handle, k.as_ptr()) };
        if p.is_null() {
            return None;
        }
        let s = unsafe { CStr::from_ptr(p).to_string_lossy().into_owned() };
        unsafe { (self.inner.api.free)(p as *mut c_void) };
        Some(s)
    }

    pub fn get_f64(&self, name: &str) -> Option<f64> {
        self.get_string(name)?.trim().replace(',', ".").parse().ok()
    }

    pub fn get_i64(&self, name: &str) -> Option<i64> {
        self.get_string(name)?.trim().parse().ok()
    }

    pub fn get_bool(&self, name: &str) -> Option<bool> {
        match self.get_string(name)?.as_str() {
            "yes" | "true" => Some(true),
            "no" | "false" => Some(false),
            _ => None,
        }
    }

    /// Exécute une commande mpv (ex. `["loadfile", "x.mkv", "replace"]`).
    pub fn command(&self, args: &[&str]) -> Result<(), String> {
        let owned: Vec<CString> = args.iter().map(|a| CString::new(*a).map_err(|e| e.to_string())).collect::<Result<_, _>>()?;
        let mut ptrs: Vec<*const c_char> = owned.iter().map(|c| c.as_ptr()).collect();
        ptrs.push(std::ptr::null());
        let rc = unsafe { (self.inner.api.command)(self.inner.handle, ptrs.as_mut_ptr()) };
        if rc < 0 { Err(format!("{}: {}", args.first().copied().unwrap_or(""), self.err(rc))) } else { Ok(()) }
    }

    /// Lit un événement (timeout en secondes, 0 = sans attente).
    pub fn wait_event(&self, timeout: f64) -> Event {
        let ev = unsafe { (self.inner.api.wait_event)(self.inner.handle, timeout) };
        if ev.is_null() {
            return Event::None;
        }
        let ev = unsafe { &*ev };
        match ev.event_id {
            0 => Event::None,
            1 => Event::Shutdown,
            6 => Event::StartFile,
            7 => {
                let reason = if ev.data.is_null() {
                    EndReason::Other
                } else {
                    match unsafe { (*(ev.data as *const RawEndFile)).reason } {
                        0 => EndReason::Eof,
                        2 => EndReason::Stop,
                        3 => EndReason::Quit,
                        4 => EndReason::Error,
                        _ => EndReason::Other,
                    }
                };
                Event::EndFile(reason)
            }
            8 => Event::FileLoaded,
            17 => Event::VideoReconfig,
            21 => Event::PlaybackRestart,
            other => Event::Other(other),
        }
    }

    /// Crée le contexte de rendu OpenGL. À appeler avec le contexte GL courant.
    /// `get_proc` résout les fonctions OpenGL (utilisée pendant la création uniquement).
    pub unsafe fn gl_render_context(&self, get_proc: &dyn Fn(&CStr) -> *const c_void) -> Result<RenderContext, String> {
        unsafe extern "C" fn tramp(ctx: *mut c_void, name: *const c_char) -> *mut c_void {
            let f = &*(ctx as *const &dyn Fn(&CStr) -> *const c_void);
            f(CStr::from_ptr(name)) as *mut c_void
        }
        let f: &dyn Fn(&CStr) -> *const c_void = get_proc;
        let mut init = OglInit { get_proc_address: Some(tramp), ctx: &f as *const _ as *mut c_void };
        let api_type = CString::new("opengl").unwrap();
        let mut params = [
            RenderParam { kind: R_API_TYPE, data: api_type.as_ptr() as *mut c_void },
            RenderParam { kind: R_OPENGL_INIT, data: &mut init as *mut _ as *mut c_void },
            RenderParam { kind: 0, data: std::ptr::null_mut() },
        ];
        let mut ctx: *mut c_void = std::ptr::null_mut();
        let rc = (self.inner.api.render_create)(&mut ctx, self.inner.handle, params.as_mut_ptr());
        if rc < 0 || ctx.is_null() {
            return Err(format!("mpv_render_context_create : {}", self.err(rc)));
        }
        Ok(RenderContext { inner: self.inner.clone(), ctx, callback: None })
    }
}

/// Contexte de rendu OpenGL de mpv.
pub struct RenderContext {
    inner: Arc<Inner>,
    ctx: *mut c_void,
    callback: Option<Box<Box<dyn Fn() + Send + Sync>>>,
}
unsafe impl Send for RenderContext {}

impl RenderContext {
    /// Fonction appelée (depuis un thread mpv) quand une nouvelle image est prête : ne doit pas appeler mpv.
    pub fn set_update_callback(&mut self, f: impl Fn() + Send + Sync + 'static) {
        unsafe extern "C" fn tramp(p: *mut c_void) {
            let f = &*(p as *const Box<dyn Fn() + Send + Sync>);
            f();
        }
        let boxed: Box<Box<dyn Fn() + Send + Sync>> = Box::new(Box::new(f));
        let ptr = &*boxed as *const Box<dyn Fn() + Send + Sync> as *mut c_void;
        unsafe { (self.inner.api.render_set_callback)(self.ctx, Some(tramp), ptr) };
        self.callback = Some(boxed);
    }

    /// Drapeaux de mise à jour (bit 0 = nouvelle image à dessiner).
    pub fn update(&self) -> u64 {
        unsafe { (self.inner.api.render_update)(self.ctx) }
    }

    /// Dessine l'image courante dans le framebuffer `fbo` de taille `w` x `h`.
    pub fn render(&self, fbo: i32, w: i32, h: i32, flip_y: bool) -> Result<(), String> {
        let mut f = OglFbo { fbo, w, h, internal_format: 0 };
        let mut flip: c_int = flip_y as c_int;
        let mut params = [
            RenderParam { kind: R_OPENGL_FBO, data: &mut f as *mut _ as *mut c_void },
            RenderParam { kind: R_FLIP_Y, data: &mut flip as *mut _ as *mut c_void },
            RenderParam { kind: 0, data: std::ptr::null_mut() },
        ];
        let rc = unsafe { (self.inner.api.render_render)(self.ctx, params.as_mut_ptr()) };
        if rc < 0 { Err(format!("render : {rc}")) } else { Ok(()) }
    }
}

impl Drop for RenderContext {
    fn drop(&mut self) {
        // Doit être libéré avant mpv_terminate_destroy (le Arc<Inner> est libéré après ce Drop).
        unsafe { (self.inner.api.render_free)(self.ctx) };
    }
}

/// Utile pour un test de chargement sans lecteur : la bibliothèque est-elle présente ?
pub fn library_available() -> Result<(), String> {
    load().map(|_| ())
}
