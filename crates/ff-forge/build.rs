//! Icône MediaForge et métadonnées du .exe (Windows uniquement). v0.4.0
fn main() {
    println!("cargo:rerun-if-changed=../../assets/fileforge.ico");
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../assets/fileforge.ico");
        res.set("ProductName", "FileForge");
        res.set("FileDescription", "FileForge");
        if let Err(e) = res.compile() {
            // Ne bloque pas la compilation si le compilateur de ressources est absent.
            println!("cargo:warning=icône non intégrée : {e}");
        }
    }
}
