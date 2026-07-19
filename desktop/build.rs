//! Windows builds embed the logo into the executable, which is where the
//! taskbar and Explorer take the app icon from. Other platforms get the
//! icon at runtime (X11, Windows titlebar) or from the surrounding
//! bundle's desktop entry (Wayland); macOS would need an .app bundle,
//! which the larger bundle owns.

fn main() {
    println!("cargo:rerun-if-changed=assets/emiu2.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/emiu2.ico");
        if let Err(why) = res.compile() {
            println!("cargo:warning=could not embed the app icon: {why}");
        }
    }
}
