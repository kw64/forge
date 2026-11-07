//! Icône MediaForge et métadonnées du .exe (Windows uniquement). v0.4.0
fn main() {
    println!("cargo:rerun-if-changed=../../assets/mediaforge.ico");
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../assets/mediaforge.ico");
        res.set("ProductName", "MediaForge");
        res.set("FileDescription", "MediaForge");
        if let Err(e) = res.compile() {
            // Ne bloque pas la compilation si le compilateur de ressources est absent.
            println!("cargo:warning=icône non intégrée : {e}");
        }
    }
}
