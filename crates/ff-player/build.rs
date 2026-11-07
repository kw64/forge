//! Icône MediaForge et métadonnées du .exe (Windows uniquement). v0.4.0
fn main() {
    println!("cargo:rerun-if-changed=../../assets/mediaforge-player.ico");
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../assets/mediaforge-player.ico");
        res.set("ProductName", "MediaForge Player");
        res.set("FileDescription", "MediaForge Player");
        if let Err(e) = res.compile() {
            // Ne bloque pas la compilation si le compilateur de ressources est absent.
            println!("cargo:warning=icône non intégrée : {e}");
        }
    }
}
