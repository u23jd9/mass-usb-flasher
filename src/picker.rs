use std::path::PathBuf;
#[cfg(target_os = "linux")]
use std::process::Command;

/// Attempts to pick a disk image file (.img, .iso, .raw) using native desktop dialogs.
/// On KDE: tries `kdialog` first.
/// On GNOME/GTK: tries `zenity` first.
/// Fallback: `rfd::FileDialog`.
pub fn pick_image_file() -> Option<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        let desktop = std::env::var("XDG_CURRENT_DESKTOP")
            .unwrap_or_default()
            .to_uppercase();

        if desktop.contains("KDE") {
            if let Some(path) = pick_file_kdialog() {
                return Some(path);
            }
            if let Some(path) = pick_file_zenity() {
                return Some(path);
            }
        } else {
            if let Some(path) = pick_file_zenity() {
                return Some(path);
            }
            if let Some(path) = pick_file_kdialog() {
                return Some(path);
            }
        }
    }

    // Try rfd (works on Windows, macOS, and Linux portals)
    rfd::FileDialog::new()
        .set_title("Select image")
        .add_filter("Disk Images", &["img", "iso", "raw", "bin", "gz", "xz", "zst"])
        .add_filter("All files", &["*"])
        .pick_file()
}

/// Attempts to pick a folder using native desktop dialogs.
pub fn pick_image_folder() -> Option<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        let desktop = std::env::var("XDG_CURRENT_DESKTOP")
            .unwrap_or_default()
            .to_uppercase();

        if desktop.contains("KDE") {
            if let Some(path) = pick_folder_kdialog() {
                return Some(path);
            }
            if let Some(path) = pick_folder_zenity() {
                return Some(path);
            }
        } else {
            if let Some(path) = pick_folder_zenity() {
                return Some(path);
            }
            if let Some(path) = pick_folder_kdialog() {
                return Some(path);
            }
        }
    }

    rfd::FileDialog::new()
        .set_title("Select folder with images")
        .pick_folder()
}

#[cfg(target_os = "linux")]
fn apply_desktop_env(cmd: &mut Command) {
    if let Ok(d) = std::env::var("DISPLAY") {
        cmd.env("DISPLAY", d);
    }
    if let Ok(w) = std::env::var("WAYLAND_DISPLAY") {
        cmd.env("WAYLAND_DISPLAY", w);
    }
    if let Ok(r) = std::env::var("XDG_RUNTIME_DIR") {
        cmd.env("XDG_RUNTIME_DIR", r);
    }
    if let Ok(b) = std::env::var("DBUS_SESSION_BUS_ADDRESS") {
        cmd.env("DBUS_SESSION_BUS_ADDRESS", b);
    }
}

#[cfg(target_os = "linux")]
fn clean_picked_path(s: &str) -> Option<PathBuf> {
    let mut clean = s.trim();
    if let Some(stripped) = clean.strip_prefix("file://") {
        clean = stripped;
    }
    let p = PathBuf::from(clean);
    if p.exists() {
        return Some(p);
    }
    // Simple percent-decode in case kdialog returns percent-encoded URLs like %20
    let mut decoded = String::new();
    let mut chars = clean.chars();
    while let Some(c) = chars.next() {
        if c == '%' {
            let h1 = chars.next();
            let h2 = chars.next();
            if let (Some(h1), Some(h2)) = (h1, h2) {
                let hex_str = format!("{}{}", h1, h2);
                if let Ok(byte) = u8::from_str_radix(&hex_str, 16) {
                    decoded.push(byte as char);
                    continue;
                } else {
                    decoded.push('%');
                    decoded.push(h1);
                    decoded.push(h2);
                    continue;
                }
            }
        }
        decoded.push(c);
    }
    let p_dec = PathBuf::from(&decoded);
    if p_dec.exists() {
        return Some(p_dec);
    }
    None
}

#[cfg(target_os = "linux")]
fn pick_file_kdialog() -> Option<PathBuf> {
    let mut cmd = Command::new("kdialog");
    cmd.args(["--title", "Select Image", "--getopenfilename"]);

    let start_dir = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    cmd.arg(&start_dir);
    cmd.arg("*.img *.iso *.raw *.bin *.gz *.xz *.zst|Disk Images (*.img *.iso *.raw *.bin)\n*|All Files (*)");

    apply_desktop_env(&mut cmd);

    let output = cmd.output().ok()?;
    if output.status.success() {
        let path_str = String::from_utf8_lossy(&output.stdout);
        return clean_picked_path(&path_str);
    }
    None
}

#[cfg(target_os = "linux")]
fn pick_folder_kdialog() -> Option<PathBuf> {
    let mut cmd = Command::new("kdialog");
    cmd.args(["--title", "Select Folder", "--getexistingdirectory"]);

    let start_dir = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    cmd.arg(&start_dir);

    apply_desktop_env(&mut cmd);

    let output = cmd.output().ok()?;
    if output.status.success() {
        let path_str = String::from_utf8_lossy(&output.stdout);
        if let Some(clean) = clean_picked_path(&path_str) {
            if clean.is_dir() {
                return Some(clean);
            }
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn pick_file_zenity() -> Option<PathBuf> {
    let mut cmd = Command::new("zenity");
    cmd.args([
        "--file-selection",
        "--title=Select Image",
        "--file-filter=Disk Images (*.img, *.iso, *.raw, *.bin) | *.img *.iso *.raw *.bin *.gz *.xz *.zst",
        "--file-filter=All files | *",
    ]);

    apply_desktop_env(&mut cmd);

    let output = cmd.output().ok()?;
    if output.status.success() {
        let path_str = String::from_utf8_lossy(&output.stdout);
        return clean_picked_path(&path_str);
    }
    None
}

#[cfg(target_os = "linux")]
fn pick_folder_zenity() -> Option<PathBuf> {
    let mut cmd = Command::new("zenity");
    cmd.args([
        "--file-selection",
        "--directory",
        "--title=Select Folder",
    ]);

    apply_desktop_env(&mut cmd);

    let output = cmd.output().ok()?;
    if output.status.success() {
        let path_str = String::from_utf8_lossy(&output.stdout);
        if let Some(clean) = clean_picked_path(&path_str) {
            if clean.is_dir() {
                return Some(clean);
            }
        }
    }
    None
}
