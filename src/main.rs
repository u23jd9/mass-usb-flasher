#![allow(dead_code)]

mod app;
mod catalog;
mod device;
mod engine;
mod i18n;
mod image;
mod taskbar;
mod ui;

use std::io::{self, Stdout};
use std::path::PathBuf;
use std::process::Command;
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use ratatui::{
    backend::CrosstermBackend,
    crossterm::{
        event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
        execute,
        terminal::{
            disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen, SetTitle,
        },
    },
    Terminal,
};

use app::{AppState, AppStatus, HashJob, Screen, SlotStatus};
use catalog::{spawn_folder_import, unix_now, ImageEntry, ImportProgress};
use device::watcher::{DeviceFilter, WatcherConfig, WatcherHandle};
use engine::writer::{start_write, WriteEvent};
use i18n::{VERSION, WATERMARK, WATERMARK_URL};
use image::{compute_sha256_async, HashProgress};
use taskbar::Taskbar;

pub enum PrepareResult {
    Log(String),
    Ready {
        write_rx: Receiver<WriteEvent>,
        opened_numbers: Vec<u32>,
        files_count: usize,
    },
    Failed(String),
}

fn main() -> io::Result<()> {
    install_panic_hook();

    if !is_admin() {
        eprintln!("[!] Administrator rights required. Run as administrator.");
        std::process::exit(1);
    }

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let res = run(&mut terminal);

    let _ = disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);
    let _ = terminal.show_cursor();

    if let Ok(state) = &res {
        print_final_log(state);
    }

    print_watermark_clickable();

    res.map(|_| ())
}

fn print_final_log(state: &AppState) {
    println!(
        "\n=== Final log ({} lines, last 40) ===",
        state.log.len()
    );
    for line in state.log.iter().rev().take(40).collect::<Vec<_>>().iter().rev() {
        println!("{}", line);
    }
}

/// Печатает вотермарку с OSC 8 гиперссылкой (кликабельно в Windows Terminal,
/// iTerm2, WezTerm и большинстве современных терминалов).
fn print_watermark_clickable() {
    println!();
    // OSC 8: \x1b]8;;URL\x1b\\TEXT\x1b]8;;\x1b\\
    println!(
        "\x1b]8;;{}\x1b\\{}\x1b]8;;\x1b\\  ·  Mass USB Flasher v{}",
        WATERMARK_URL, WATERMARK, VERSION
    );
    println!(
        "Link: \x1b]8;;{}\x1b\\{}\x1b]8;;\x1b\\",
        WATERMARK_URL, WATERMARK_URL
    );
}

fn install_panic_hook() {
    let original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        original(info);
    }));
}

fn ensure_automount_enabled() -> Result<(), String> {
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
    Ok(())
}

fn run(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<AppState> {
    let mut state = AppState::new();

    if let Some(n) = parse_target_count_arg() {
        state.set_target_count(n);
    }

    match ensure_automount_enabled() {
        Ok(()) => state.push_log("mountvol /e — automount enabled"),
        Err(e) => state.push_log(format!("[!] mountvol /e failed: {}", e)),
    }

    state.catalog = catalog::Catalog::load_or_default(&state.catalog_path);
    state.push_log(format!(
        "Catalog loaded: {} entries ({})",
        state.catalog.images.len(),
        state.catalog_path.display()
    ));

    let filter = DeviceFilter {
        usb_only: true,
        min_size_bytes: 4 * 1024 * 1024 * 1024,
        max_size_bytes: 128 * 1024 * 1024 * 1024,
    };
    let handle = device::watcher::spawn(WatcherConfig {
        poll_interval: Duration::from_millis(500),
        max_disks: 16,
        filter,
    });

    if let Some(path) = parse_args() {
        load_image_from_cli(&mut state, path);
    }

    let taskbar = Taskbar::new();

    // Устанавливаем стартовый заголовок окна.
    set_window_title(&state)?;

    let mut last_title = String::new();
    let mut last_taskbar_update = Instant::now();
    let mut last_taskbar_pct: i32 = -1;

    loop {
        drain_device_events(&mut state, &handle);
        drain_hash_job(&mut state);
        drain_import_job(&mut state);
        drain_prepare(&mut state);
        drain_write(&mut state);

        terminal.draw(|f| ui::render(f, &state))?;

        // Обновляем заголовок окна при изменении.
        let new_title = compute_window_title(&state);
        if new_title != last_title {
            let mut out = io::stdout();
            let _ = execute!(out, SetTitle(&new_title));
            last_title = new_title;
        }

        // Обновляем taskbar-прогресс (не чаще 100 мс).
                // Обновляем taskbar-прогресс (не чаще 100 мс).
        if last_taskbar_update.elapsed() >= Duration::from_millis(100) {
            let total = state.write_total_bytes;
            let done = state.slowest_bytes_written();
            if total > 0 && matches!(state.status, AppStatus::Running) {
                let pct = ((done as f64 / total as f64) * 100.0)
                    .clamp(0.0, 100.0)
                    .round() as i32;
                if pct != last_taskbar_pct {
                    taskbar.set(done, total);
                    last_taskbar_pct = pct;
                }
            } else if last_taskbar_pct >= 0 {
                taskbar.clear();
                last_taskbar_pct = -1;
            }
            last_taskbar_update = Instant::now();
        }

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if handle_key(&mut state, key) {
                    break;
                }
            }
        }

        auto_transition_finished(&mut state);
    }

    // Сбрасываем прогресс при выходе.
    taskbar.clear();

    Ok(state)
}

fn set_window_title(state: &AppState) -> io::Result<()> {
    let title = compute_window_title(state);
    let mut out = io::stdout();
    execute!(out, SetTitle(&title))?;
    Ok(())
}

fn compute_window_title(state: &AppState) -> String {
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
        // Прогресс считаем по самой отстающей флешке — когда она дойдёт
        // до конца, вся операция завершена.
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

// --- Каналы ---

fn drain_device_events(state: &mut AppState, handle: &WatcherHandle) {
    while let Ok(ev) = handle.events.try_recv() {
        state.apply_device_event(ev);
    }
}

fn drain_hash_job(state: &mut AppState) {
    let Some(job) = state.hash_job.take() else {
        return;
    };
    let mut keep = true;

    loop {
        match job.rx.try_recv() {
            Ok(HashProgress::Progress { bytes_done, bytes_total }) => {
                state.image_hash_progress = Some((bytes_done, bytes_total));
            }
            Ok(HashProgress::Done { size_bytes, sha256 }) => {
                state.image_sha256 = Some(sha256.clone());
                state.image_hash_progress = None;
                state.push_log(format!("SHA-256: {}", sha256));
                if job.save_to_catalog {
                    save_current_image_to_catalog(state, size_bytes, sha256);
                }
                keep = false;
                break;
            }
            Ok(HashProgress::Error(e)) => {
                state.push_log(format!("[!] Image read error: {}", e));
                state.image_hash_progress = None;
                keep = false;
                break;
            }
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => {
                keep = false;
                break;
            }
        }
    }

    if keep {
        state.hash_job = Some(job);
    }
}

fn save_current_image_to_catalog(state: &mut AppState, size_bytes: u64, sha256: String) {
    let (Some(work_name), Some(path)) = (state.image_work_name.clone(), state.image_path.clone())
    else {
        return;
    };

    state.catalog.upsert(ImageEntry {
        work_name,
        path,
        size_bytes,
        sha256,
        added_at_unix: unix_now(),
    });

    match state.catalog.save(&state.catalog_path) {
        Ok(()) => state.push_log("Image added to catalog"),
        Err(e) => state.push_log(format!("[!] Catalog save failed: {}", e)),
    }
}

fn drain_import_job(state: &mut AppState) {
    let Some(rx) = state.import_job.take() else {
        return;
    };
    let mut keep = true;

    loop {
        match rx.try_recv() {
            Ok(p @ ImportProgress::Scanning { total }) => {
                state.push_log(format!("[import] scanning, files: {}", total));
                state.import_progress = Some(p);
            }
            Ok(p @ ImportProgress::HashingFile { .. }) => {
                state.import_progress = Some(p);
            }
            Ok(ImportProgress::FileDone { name }) => {
                state.push_log(format!("[import] {}", name));
            }
            Ok(ImportProgress::Done { imported, entries }) => {
                state.push_log(format!("[import] done, imported: {}", imported));
                for e in entries {
                    state.catalog.upsert(e);
                }
                match state.catalog.save(&state.catalog_path) {
                    Ok(()) => state.push_log("Catalog updated"),
                    Err(e) => state.push_log(format!("[!] Catalog: {}", e)),
                }
                state.import_progress = None;
                keep = false;
                break;
            }
            Ok(ImportProgress::Error(e)) => {
                state.push_log(format!("[!] Import: {}", e));
                state.import_progress = None;
                keep = false;
                break;
            }
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => {
                state.import_progress = None;
                keep = false;
                break;
            }
        }
    }

    if keep {
        state.import_job = Some(rx);
    }
}

fn drain_prepare(state: &mut AppState) {
    let Some(rx) = state.prepare_rx.take() else {
        return;
    };
    let mut keep = true;

    loop {
        match rx.try_recv() {
            Ok(PrepareResult::Log(msg)) => state.push_log(msg),
            Ok(PrepareResult::Ready {
                write_rx,
                opened_numbers,
                files_count,
            }) => {
                state.mark_slots_writing(&opened_numbers);
                state.write_started_at = Some(std::time::Instant::now());
                state.push_log(format!("=== Writing to {} devices ===", files_count));
                state.write_rx = Some(write_rx);
                keep = false;
                break;
            }
            Ok(PrepareResult::Failed(e)) => {
                state.push_log(format!("[!] Prepare failed: {}", e));
                state.status = AppStatus::Ready;
                state.write_started_at = None;
                state.rollback_slots_to_ready();
                keep = false;
                break;
            }
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => {
                keep = false;
                break;
            }
        }
    }

    if keep {
        state.prepare_rx = Some(rx);
    }
}

fn drain_write(state: &mut AppState) {
    let Some(rx) = state.write_rx.take() else {
        return;
    };
    let mut keep = true;

    loop {
        match rx.try_recv() {
            Ok(WriteEvent::Progress { id, bytes_written }) => {
                state.update_progress_by_index(id, bytes_written, state.write_total_bytes);
            }
            Ok(WriteEvent::Completed { id }) => {
                state.mark_completed_by_index(id);
                state.push_log(format!("[{}] write complete", id));
            }
            Ok(WriteEvent::Errored { id, message }) => {
                state.mark_failed_by_index(id, message.clone());
                state.push_log(format!("[!] [{}] error: {}", id, message));
            }
            Ok(WriteEvent::Finished { result }) => {
                match result {
                    Ok(()) => state.push_log("=== Write complete ==="),
                    Err(e) => state.push_log(format!("[!] Write finished with error: {}", e)),
                }
                return_devices_online(state);
                state.status = AppStatus::Finished;
                keep = false;
                break;
            }
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => {
                state.push_log("[!] Write channel disconnected");
                state.status = AppStatus::Finished;
                keep = false;
                break;
            }
        }
    }

    if keep {
        state.write_rx = Some(rx);
    }
}

fn return_devices_online(state: &mut AppState) {
    let numbers: Vec<u32> = state.devices.keys().copied().collect();
    for n in numbers {
        match engine::set_disk_offline(n, false) {
            Ok(()) => state.push_log(format!("[#{}] back online", n)),
            Err(e) => state.push_log(format!("[#{}] failed to bring online: {}", n, e)),
        }
    }
}

fn auto_transition_finished(state: &mut AppState) {
    if state.status != AppStatus::Running || state.devices.is_empty() {
        return;
    }
    let all_done = state
        .devices
        .values()
        .all(|s| matches!(s.status, SlotStatus::Done | SlotStatus::Failed));
    if all_done {
        state.status = AppStatus::Finished;
    }
}

// --- Клавиатура ---

fn handle_key(state: &mut AppState, key: KeyEvent) -> bool {
    if key.kind != KeyEventKind::Press {
        return false;
    }

    if key.modifiers.contains(KeyModifiers::CONTROL) {
        if matches!(
            key.code,
            KeyCode::Char('c') | KeyCode::Char('C') | KeyCode::Char('с') | KeyCode::Char('С')
        ) {
            return true;
        }
    }

    match state.screen {
        Screen::Main => handle_key_main(state, key),
        Screen::ImageSelect => handle_key_image_select(state, key),
        Screen::AddImageName => handle_key_add_name(state, key),
        Screen::RenameImage => handle_key_rename(state, key),
        Screen::Help => handle_key_help(state, key),
    }
}

fn handle_key_main(state: &mut AppState, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Char('+') | KeyCode::Char('=') => {
            state.increase_target();
            state.push_log(format!("Target: {}", state.target_count));
            return false;
        }
        KeyCode::Char('-') => {
            state.decrease_target();
            state.push_log(format!("Target: {}", state.target_count));
            return false;
        }
        _ => {}
    }

    let KeyCode::Char(c) = key.code else {
        return false;
    };
    match normalize_key(c) {
        'q' => return true,
        's' => start_write_flow(state),
        'i' => {
            state.screen = Screen::ImageSelect;
            state.image_select_index = 0;
            state.pending_image_path = None;
            state.push_log("Image catalog opened");
        }
        'h' => {
            state.screen = Screen::Help;
        }
        'l' => {
            state.lang = state.lang.toggle();
            let msg = format!("Language: {}", state.lang.code());
            state.push_log(msg);
        }
        'v' => state.push_log("Verify (stub)"),
        'r' => state.push_log("Reformat (stub)"),
        _ => {}
    }
    false
}

fn handle_key_image_select(state: &mut AppState, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Esc => {
            state.screen = Screen::Main;
        }
        KeyCode::Up => {
            state.image_select_index = state.image_select_index.saturating_sub(1);
        }
        KeyCode::Down => {
            let n = state.catalog.images.len();
            if n > 0 && state.image_select_index + 1 < n {
                state.image_select_index += 1;
            }
        }
        KeyCode::Enter => {
            let idx = state.image_select_index;
            if idx < state.catalog.images.len() {
                match state.select_image_from_catalog(idx) {
                    Ok(()) => {
                        let name = state.image_work_name.clone().unwrap_or_default();
                        state.push_log(format!("Selected: {}", name));
                        state.screen = Screen::Main;
                    }
                    Err(e) => state.push_log(format!("[!] {}", e)),
                }
            }
        }
        KeyCode::Char(c) => match normalize_key(c) {
            'a' => open_add_file_dialog(state),
            'o' => open_ephemeral_file_dialog(state),
            'd' => delete_selected_from_catalog(state),
            'r' => start_rename_selected(state),
            'f' => open_import_folder_dialog(state),
            _ => {}
        },
        _ => {}
    }
    false
}

fn handle_key_help(state: &mut AppState, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Char(_) | KeyCode::Esc | KeyCode::Enter => {
            state.screen = Screen::Main;
        }
        _ => {}
    }
    false
}

fn handle_key_add_name(state: &mut AppState, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Esc => {
            state.pending_image_path = None;
            state.input_buffer.clear();
            state.screen = Screen::ImageSelect;
        }
        KeyCode::Enter => commit_new_image(state),
        KeyCode::Backspace => {
            state.input_buffer.pop();
        }
        KeyCode::Char(c) => push_name_char(state, c),
        _ => {}
    }
    false
}

fn handle_key_rename(state: &mut AppState, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Esc => {
            state.rename_target_index = None;
            state.input_buffer.clear();
            state.screen = Screen::ImageSelect;
        }
        KeyCode::Enter => commit_rename(state),
        KeyCode::Backspace => {
            state.input_buffer.pop();
        }
        KeyCode::Char(c) => push_name_char(state, c),
        _ => {}
    }
    false
}

fn push_name_char(state: &mut AppState, c: char) {
    if (c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | ' '))
        && state.input_buffer.len() < 64
    {
        state.input_buffer.push(c);
    }
}

// --- Действия ---

fn open_add_file_dialog(state: &mut AppState) {
    let Some(path) = rfd::FileDialog::new()
        .set_title("Select image")
        .add_filter("Images", &["img", "iso", "raw"])
        .add_filter("All files", &["*"])
        .pick_file()
    else {
        return;
    };

    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    let default_name = state
        .find_catalog_entry_by_path(&path)
        .map(|e| e.work_name.clone())
        .unwrap_or_else(|| {
            path.file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default()
        });

    state.pending_image_path = Some(path.clone());
    state.pending_image_size = size;
    state.input_buffer = default_name;
    state.screen = Screen::AddImageName;
    state.push_log(format!(
        "Adding image: {} ({:.2} GB)",
        path.display(),
        size as f64 / 1e9
    ));
}

fn open_ephemeral_file_dialog(state: &mut AppState) {
    let Some(path) = rfd::FileDialog::new()
        .set_title("Open image once (without saving to catalog)")
        .add_filter("Images", &["img", "iso", "raw"])
        .add_filter("All files", &["*"])
        .pick_file()
    else {
        return;
    };

    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    let name = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "image".to_string());

    state.image_path = Some(path.clone());
    state.image_size = size;
    state.image_work_name = Some(name.clone());
    state.image_sha256 = None;
    state.image_hash_progress = None;

    state.push_log(format!(
        "Opened once: {} ({:.2} GB)",
        path.display(),
        size as f64 / 1e9
    ));
    state.hash_job = Some(HashJob {
        rx: compute_sha256_async(path.display().to_string()),
        save_to_catalog: false,
    });

    state.screen = Screen::Main;
}

fn open_import_folder_dialog(state: &mut AppState) {
    if state.import_job.is_some() {
        state.push_log("[!] Import already in progress");
        return;
    }
    let Some(folder) = rfd::FileDialog::new()
        .set_title("Select folder with images")
        .pick_folder()
    else {
        return;
    };
    state.push_log(format!("[import] folder: {}", folder.display()));
    state.import_job = Some(spawn_folder_import(folder));
}

fn delete_selected_from_catalog(state: &mut AppState) {
    let idx = state.image_select_index;
    let Some(entry) = state.catalog.images.get(idx) else {
        return;
    };
    let name = entry.work_name.clone();

    if state.catalog.remove_by_name(&name) {
        match state.catalog.save(&state.catalog_path) {
            Ok(()) => state.push_log(format!("Removed: {}", name)),
            Err(e) => state.push_log(format!("[!] Catalog: {}", e)),
        }
        if state.image_select_index >= state.catalog.images.len() && state.image_select_index > 0 {
            state.image_select_index -= 1;
        }
    }
}

fn start_rename_selected(state: &mut AppState) {
    let idx = state.image_select_index;
    let Some(entry) = state.catalog.images.get(idx) else {
        return;
    };
    state.input_buffer = entry.work_name.clone();
    state.rename_target_index = Some(idx);
    state.screen = Screen::RenameImage;
}

fn commit_rename(state: &mut AppState) {
    let new_name = state.input_buffer.trim().to_string();
    if new_name.is_empty() {
        return;
    }
    let Some(idx) = state.rename_target_index.take() else {
        return;
    };
    let Some(old_name) = state
        .catalog
        .images
        .get(idx)
        .map(|e| e.work_name.clone())
    else {
        state.screen = Screen::ImageSelect;
        return;
    };

    match state.rename_in_catalog(&old_name, &new_name) {
        Ok(()) => {
            match state.catalog.save(&state.catalog_path) {
                Ok(()) => state.push_log(format!("Renamed '{}' → '{}'", old_name, new_name)),
                Err(e) => state.push_log(format!("[!] Catalog: {}", e)),
            }
            state.screen = Screen::ImageSelect;
        }
        Err(e) => {
            state.push_log(format!("[!] {}", e));
            state.rename_target_index = Some(idx);
        }
    }
    state.input_buffer.clear();
}

fn commit_new_image(state: &mut AppState) {
    let work_name = state.input_buffer.trim().to_string();
    if work_name.is_empty() {
        return;
    }
    let Some(path) = state.pending_image_path.take() else {
        return;
    };

    state.image_path = Some(path.clone());
    state.image_size = state.pending_image_size;
    state.image_work_name = Some(work_name.clone());
    state.image_sha256 = None;
    state.image_hash_progress = None;

    state.push_log(format!("Computing SHA-256 for '{}'…", work_name));
    state.hash_job = Some(HashJob {
        rx: compute_sha256_async(path.display().to_string()),
        save_to_catalog: true,
    });

    state.screen = Screen::Main;
    state.input_buffer.clear();
}

fn load_image_from_cli(state: &mut AppState, path: PathBuf) {
    if !path.exists() {
        state.push_log(format!("[!] File not found: {}", path.display()));
        return;
    }
    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    state.image_path = Some(path.clone());
    state.image_size = size;

    if let Some(entry) = state.find_catalog_entry_by_path(&path).cloned() {
        state.image_work_name = Some(entry.work_name.clone());
        state.image_sha256 = Some(entry.sha256);
        state.push_log(format!("Found in catalog: {} (SHA-256 ready)", entry.work_name));
        return;
    }

    state.push_log(format!(
        "Computing SHA-256: {} ({:.2} GB)…",
        path.display(),
        size as f64 / 1e9
    ));
    state.hash_job = Some(HashJob {
        rx: compute_sha256_async(path.display().to_string()),
        save_to_catalog: false,
    });
}

// --- Запись ---

fn start_write_flow(state: &mut AppState) {
    if state.image_path.is_none() {
        state.push_log("[!] Select an image first (I)");
        return;
    }
    if state.image_sha256.is_none() {
        state.push_log("[!] SHA-256 not yet computed");
        return;
    }

    let device_numbers = state.ready_device_numbers();
    if device_numbers.is_empty() {
        state.push_log("[!] No devices in Ready state");
        return;
    }
    if device_numbers.len() < state.target_count {
        state.push_log(format!(
            "[!] Need {} devices, ready {}",
            state.target_count,
            device_numbers.len()
        ));
        return;
    }

    if state.status == AppStatus::Running || state.prepare_rx.is_some() {
        state.push_log("[!] Operation already in progress");
        return;
    }

    let image_path = state.image_path.as_ref().unwrap().display().to_string();
    let total = state.image_size;

    state.status = AppStatus::Running;
    state.write_total_bytes = total;
    state.write_started_at = None;
    state.push_log(format!("Preparing {} devices…", device_numbers.len()));

    let (tx, rx) = channel();
    state.prepare_rx = Some(rx);

    thread::spawn(move || prepare_and_start(tx, device_numbers, image_path, total));
}

fn prepare_and_start(
    tx: std::sync::mpsc::Sender<PrepareResult>,
    device_numbers: Vec<u32>,
    image_path: String,
    total: u64,
) {
    let mut opened: Vec<(u32, std::fs::File)> = Vec::new();

    for n in device_numbers {
        let _ = tx.send(PrepareResult::Log(format!("[#{}] unmounting volumes…", n)));
        match engine::unmount_all_volumes_on_disk(n) {
            Ok(0) => {
                let _ = tx.send(PrepareResult::Log(format!("[#{}] no volumes", n)));
            }
            Ok(c) => {
                let _ = tx.send(PrepareResult::Log(format!(
                    "[#{}] volumes unmounted: {}",
                    n, c
                )));
            }
            Err(e) => {
                let _ = tx.send(PrepareResult::Log(format!(
                    "[#{}] unmount error: {}",
                    n, e
                )));
            }
        }

        let _ = tx.send(PrepareResult::Log(format!("[#{}] setting offline…", n)));
        if let Err(e) = engine::set_disk_offline(n, true) {
            let _ = tx.send(PrepareResult::Log(format!(
                "[!] #{} offline failed: {}",
                n, e
            )));
        }

        match engine::open_drive_write(n) {
            Ok(f) => {
                let _ = tx.send(PrepareResult::Log(format!("[#{}] opened for write", n)));
                opened.push((n, f));
            }
            Err(e) => {
                let _ = tx.send(PrepareResult::Log(format!(
                    "[!] #{} open failed: {}",
                    n, e
                )));
            }
        }
    }

    if opened.is_empty() {
        let _ = tx.send(PrepareResult::Failed("no devices opened".into()));
        return;
    }

    let opened_numbers: Vec<u32> = opened.iter().map(|(n, _)| *n).collect();
    let files_count = opened.len();
    let files: Vec<std::fs::File> = opened.into_iter().map(|(_, f)| f).collect();

    let write_rx = start_write(image_path, files, total);
    let _ = tx.send(PrepareResult::Ready {
        write_rx,
        opened_numbers,
        files_count,
    });
}

// --- Утилиты ---

fn normalize_key(c: char) -> char {
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

fn parse_args() -> Option<PathBuf> {
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

fn parse_target_count_arg() -> Option<usize> {
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

#[cfg(windows)]
fn is_admin() -> bool {
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
fn is_admin() -> bool {
    true
}