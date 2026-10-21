//! Embeds the app icon into the Windows executable, so Explorer, the
//! taskbar and shortcuts show it. (Other platforms set it at runtime.)

fn main() {
    println!("cargo:rerun-if-changed=assets/paint.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/paint.ico");
        res.compile().expect("failed to embed the Windows icon");
    }
}
