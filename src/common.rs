use std::path::PathBuf;
#[cfg(windows)]
use std::process::Command;

use crate::app::{AppState, AppStatus};
use crate::i18n::VERSION;

#[cfg(windows)]
pub fn is_admin() -> bool {
    use std::fs::OpenOptions;
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_SHARE_READ: u32 = 0x0000_0001;
    const FILE_SHARE_WRITE: u32 = 0x0000_0002;
    OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .open(r"\\.\PhysicalDrive0")
        .is_ok()
}

#[cfg(not(windows))]
pub fn is_admin() -> bool {
    unsafe { libc::geteuid() == 0 }
}

/// Applies environment variables passed during self-elevation.
pub fn handle_elevation_args() {
    let args: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < args.len() {
        if args[i] == "--env-wayland-display" && i + 1 < args.len() {
            std::env::set_var("WAYLAND_DISPLAY", &args[i + 1]);
            i += 1;
        } else if args[i] == "--env-xdg-runtime-dir" && i + 1 < args.len() {
            std::env::set_var("XDG_RUNTIME_DIR", &args[i + 1]);
            i += 1;
        } else if args[i] == "--env-display" && i + 1 < args.len() {
            std::env::set_var("DISPLAY", &args[i + 1]);
            i += 1;
        } else if args[i] == "--env-xauthority" && i + 1 < args.len() {
            std::env::set_var("XAUTHORITY", &args[i + 1]);
            i += 1;
        } else if args[i] == "--env-user" && i + 1 < args.len() {
            std::env::set_var("SUDO_USER", &args[i + 1]);
            i += 1;
        } else if args[i] == "--env-dbus-session" && i + 1 < args.len() {
            std::env::set_var("DBUS_SESSION_BUS_ADDRESS", &args[i + 1]);
            i += 1;
        }
        i += 1;
    }
}

/// Ensures the current process runs with administrative / root privileges.
/// If not, it requests elevation (via Polkit / pkexec in GUI or sudo in terminal).
#[cfg(not(windows))]
pub fn ensure_elevated(is_gui: bool) -> bool {
    handle_elevation_args();

    if is_admin() {
        return true;
    }

    if std::env::args().any(|a| a == "--elevated-attempt") {
        eprintln!("[!] Administrator/root rights are required to access disk devices.");
        return false;
    }

    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("[!] Failed to determine current executable: {}", e);
            return false;
        }
    };

    // Allow root to access X11 / Xwayland display if DISPLAY is set
    if std::env::var_os("DISPLAY").is_some() {
        let _ = std::process::Command::new("xhost")
            .arg("+SI:localuser:root")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }

    let mut child_args = Vec::new();
    child_args.push("--elevated-attempt".to_string());

    if let Ok(w) = std::env::var("WAYLAND_DISPLAY") {
        child_args.push("--env-wayland-display".to_string());
        child_args.push(w);
    }
    if let Ok(r) = std::env::var("XDG_RUNTIME_DIR") {
        child_args.push("--env-xdg-runtime-dir".to_string());
        child_args.push(r);
    }
    if let Ok(d) = std::env::var("DISPLAY") {
        child_args.push("--env-display".to_string());
        child_args.push(d);
    }
    if let Ok(x) = std::env::var("XAUTHORITY") {
        child_args.push("--env-xauthority".to_string());
        child_args.push(x);
    }
    if let Ok(b) = std::env::var("DBUS_SESSION_BUS_ADDRESS") {
        child_args.push("--env-dbus-session".to_string());
        child_args.push(b);
    }
    if let Ok(u) = std::env::var("USER") {
        child_args.push("--env-user".to_string());
        child_args.push(u);
    }

    for arg in std::env::args().skip(1) {
        if !arg.starts_with("--env-") && arg != "--elevated-attempt" {
            child_args.push(arg);
        }
    }

    let is_terminal = unsafe { libc::isatty(libc::STDIN_FILENO) == 1 };
    let has_pkexec = std::process::Command::new("which")
        .arg("pkexec")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    let status = if has_pkexec && (is_gui || !is_terminal) {
        std::process::Command::new("pkexec")
            .arg(&exe)
            .args(&child_args)
            .status()
    } else {
        std::process::Command::new("sudo")
            .arg("-E")
            .arg(&exe)
            .args(&child_args)
            .status()
            .or_else(|_| {
                std::process::Command::new("sudo")
                    .arg(&exe)
                    .args(&child_args)
                    .status()
            })
    };

    match status {
        Ok(s) => {
            std::process::exit(s.code().unwrap_or(1));
        }
        Err(e) => {
            eprintln!("[!] Elevation failed: {}", e);
            false
        }
    }
}

#[cfg(windows)]
pub fn ensure_elevated(_is_gui: bool) -> bool {
    handle_elevation_args();
    is_admin()
}

pub fn ensure_automount_enabled() -> Result<(), String> {
    #[cfg(windows)]
    {
        let output = Command::new("cmd")
            .args(["/C", "mountvol", "/e"])
            .output()
            .map_err(|e| format!("failed to run mountvol: {}", e))?;

        if !output.status.success() {
            return Err(format!(
                "code {:?}, stderr: {}",
                output.status.code(),
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
    }
    Ok(())
}

pub fn normalize_key(c: char) -> char {
    let lower = c.to_lowercase().next().unwrap_or(c);
    match lower {
        'й' => 'q', 'ц' => 'w', 'у' => 'e', 'к' => 'r', 'е' => 't', 'н' => 'y',
        'г' => 'u', 'ш' => 'i', 'щ' => 'o', 'з' => 'p', 'х' => '[', 'ъ' => ']',
        'ф' => 'a', 'ы' => 's', 'в' => 'd', 'а' => 'f', 'п' => 'g', 'р' => 'h',
        'о' => 'j', 'л' => 'k', 'д' => 'l', 'ж' => ';', 'э' => '\'',
        'я' => 'z', 'ч' => 'x', 'с' => 'c', 'м' => 'v', 'и' => 'b', 'т' => 'n',
        'ь' => 'm', 'б' => ',', 'ю' => '.',
        _ => lower,
    }
}

pub fn parse_args() -> Option<PathBuf> {
    let args: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < args.len() {
        if args[i] == "--image" || args[i] == "-i" {
            if i + 1 < args.len() {
                return Some(PathBuf::from(&args[i + 1]));
            }
            eprintln!("[!] --image requires a file path");
            std::process::exit(1);
        }
        i += 1;
    }
    None
}

pub fn parse_target_count_arg() -> Option<usize> {
    let args: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < args.len() {
        if args[i] == "--target" || args[i] == "-t" {
            if i + 1 < args.len() {
                if let Ok(n) = args[i + 1].parse::<usize>() {
                    return Some(n);
                }
                eprintln!("[!] --target requires a number");
                std::process::exit(1);
            }
            eprintln!("[!] --target requires a value");
            std::process::exit(1);
        }
        i += 1;
    }
    None
}

pub fn compute_window_title(state: &AppState) -> String {
    let status = match state.status {
        AppStatus::Idle => "Idle",
        AppStatus::WaitingForDevices => "Waiting",
        AppStatus::Ready => "Ready",
        AppStatus::Running => "Running",
        AppStatus::Finished => "Finished",
    };
    let target = state.target_count;
    let active = state.active_or_ready_count();

    if state.status == AppStatus::Running && state.write_total_bytes > 0 {
        let done = state.slowest_bytes_written();
        let total = state.write_total_bytes;
        let pct = ((done as f64 / total as f64) * 100.0).clamp(0.0, 100.0).round() as u32;
        format!("Mass USB Flasher v{} — {} {}%", VERSION, status, pct)
    } else {
        format!(
            "Mass USB Flasher v{} — {} [{}/{}]",
            VERSION, status, active, target
        )
    }
}

pub fn format_elapsed(secs: f64) -> String {
    let s = secs.round() as u64;
    let m = s / 60;
    let s = s % 60;
    if m > 0 {
        format!("{}m {:02}s", m, s)
    } else {
        format!("{}s", s)
    }
}
