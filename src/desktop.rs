//! Linux desktop integration.
//!
//! GNOME, KDE and most docks ignore the icon a window sets (Wayland has no
//! way to set one at all) and look it up from the `.desktop` entry whose name
//! matches the window's app id instead. So on startup we install that entry
//! and the icons into the user's data directory (no root needed), pointing at
//! the running executable. It's rewritten only when something changed, e.g.
//! the binary moved.

use crate::APP_ID;
use std::path::{Path, PathBuf};

/// Marks entries we wrote, so we never overwrite someone else's `paint.desktop`.
const MARKER: &str = "X-Paint-Managed=true";

const ICONS: [(u32, &[u8]); 7] = [
    (16, include_bytes!("../assets/icons/16.png")),
    (24, include_bytes!("../assets/icons/24.png")),
    (32, include_bytes!("../assets/icons/32.png")),
    (48, include_bytes!("../assets/icons/48.png")),
    (64, include_bytes!("../assets/icons/64.png")),
    (128, include_bytes!("../assets/icons/128.png")),
    (256, include_bytes!("../assets/icons/256.png")),
];

/// Installs the desktop entry and icons. Failures are ignored: the app works
/// fine without them, it just shows a generic icon.
pub fn install() {
    let (Some(data), Ok(exe)) = (data_home(), std::env::current_exe()) else {
        return;
    };
    // Don't litter the desktop with throwaway test builds.
    if exe.components().any(|c| c.as_os_str() == "deps") {
        return;
    }
    let _ = install_into(&data, &exe);
}

fn data_home() -> Option<PathBuf> {
    match std::env::var_os("XDG_DATA_HOME") {
        Some(dir) if Path::new(&dir).is_absolute() => Some(dir.into()),
        _ => std::env::var_os("HOME").map(|home| Path::new(&home).join(".local/share")),
    }
}

fn install_into(data: &Path, exe: &Path) -> std::io::Result<()> {
    let icons = data.join("icons/hicolor");
    let mut icons_changed = false;
    for (size, png) in ICONS {
        let path = icons.join(format!("{size}x{size}/apps/{APP_ID}.png"));
        icons_changed |= write_if_changed(&path, png)?;
    }
    if icons_changed {
        // Icon caches notice new files by the theme directory's mtime.
        let _ = std::fs::File::open(&icons)
            .and_then(|dir| dir.set_modified(std::time::SystemTime::now()));
    }

    let entry_path = data.join(format!("applications/{APP_ID}.desktop"));
    if let Ok(existing) = std::fs::read_to_string(&entry_path)
        && !existing.contains(MARKER)
    {
        return Ok(());
    }
    write_if_changed(&entry_path, desktop_entry(exe).as_bytes())?;
    Ok(())
}

/// Writes `contents` to `path` unless it already holds exactly that.
fn write_if_changed(path: &Path, contents: &[u8]) -> std::io::Result<bool> {
    if std::fs::read(path).is_ok_and(|old| old == contents) {
        return Ok(false);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, contents)?;
    Ok(true)
}

fn desktop_entry(exe: &Path) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Paint\n\
         Comment=A simple paint program\n\
         Exec={} %f\n\
         Icon={APP_ID}\n\
         Terminal=false\n\
         StartupWMClass={APP_ID}\n\
         Categories=Graphics;2DGraphics;RasterGraphics;\n\
         MimeType=image/png;image/jpeg;image/bmp;\n\
         {MARKER}\n",
        exec_arg(&exe.to_string_lossy())
    )
}

/// Quotes a path for an `Exec=` line: the spec's argument quoting, then the
/// desktop file's own backslash escaping on top.
fn exec_arg(path: &str) -> String {
    let mut quoted = String::from("\"");
    for c in path.chars() {
        match c {
            '"' | '`' | '$' | '\\' => quoted.push_str(&format!("\\{c}")),
            '%' => quoted.push_str("%%"),
            c => quoted.push(c),
        }
    }
    quoted.push('"');
    quoted.replace('\\', "\\\\")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_quoting() {
        assert_eq!(exec_arg("/opt/paint"), "\"/opt/paint\"");
        assert_eq!(exec_arg("/a b/100%"), "\"/a b/100%%\"");
        assert_eq!(exec_arg("/x$\"y"), "\"/x\\\\$\\\\\"y\"");
    }

    #[test]
    fn installs_and_respects_foreign_entries() {
        let dir = std::env::temp_dir().join(format!("paint-desktop-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let entry = dir.join("applications/paint.desktop");

        install_into(&dir, Path::new("/usr/bin/paint")).unwrap();
        let text = std::fs::read_to_string(&entry).unwrap();
        assert!(text.contains("Exec=\"/usr/bin/paint\" %f") && text.contains("Icon=paint"));
        assert!(dir.join("icons/hicolor/256x256/apps/paint.png").exists());

        install_into(&dir, Path::new("/opt/paint")).unwrap();
        assert!(
            std::fs::read_to_string(&entry)
                .unwrap()
                .contains("/opt/paint")
        );

        std::fs::write(&entry, "[Desktop Entry]\nName=Other\n").unwrap();
        install_into(&dir, Path::new("/opt/paint")).unwrap();
        assert!(std::fs::read_to_string(&entry).unwrap().contains("Other"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
