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
