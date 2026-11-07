//! Icône et métadonnées du .exe (Windows uniquement). v0.1.0
fn main() {
    println!("cargo:rerun-if-changed=../../assets/autoforge.ico");
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../assets/autoforge.ico");
        res.set("ProductName", "AutoForge");
        res.set("FileDescription", "AutoForge");
        if let Err(e) = res.compile() {
            println!("cargo:warning=icône non intégrée : {e}");
        }
    }
}
