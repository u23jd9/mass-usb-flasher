#![allow(dead_code)]

use std::io::{self, Stdout};
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

use mass_usb_flasher::app::{AppState, AppStatus, Screen};
use mass_usb_flasher::catalog::Catalog;
use mass_usb_flasher::common::{
    compute_window_title, ensure_automount_enabled, ensure_elevated, normalize_key,
    parse_args, parse_target_count_arg,
};
use mass_usb_flasher::device::watcher::{DeviceFilter, WatcherConfig};
use mass_usb_flasher::i18n::{VERSION, WATERMARK, WATERMARK_URL};
use mass_usb_flasher::taskbar::Taskbar;
use mass_usb_flasher::ui;

fn main() -> io::Result<()> {
    install_panic_hook();

    if !ensure_elevated(false) {
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

fn print_watermark_clickable() {
    println!();
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

fn run(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<AppState> {
    let mut state = AppState::new();

    if let Some(n) = parse_target_count_arg() {
        state.set_target_count(n);
    }

    match ensure_automount_enabled() {
        Ok(()) => state.push_log("mountvol /e — automount enabled"),
        Err(e) => state.push_log(format!("[!] mountvol /e failed: {}", e)),
    }

    state.catalog = Catalog::load_or_default(&state.catalog_path);
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
    let handle = mass_usb_flasher::device::watcher::spawn(WatcherConfig {
        poll_interval: Duration::from_millis(500),
        max_disks: 16,
        filter,
    });

    if let Some(path) = parse_args() {
        state.load_image_from_cli(path);
    }

    let taskbar = Taskbar::new();

    set_window_title(&state)?;

    let mut last_title = String::new();
    let mut last_taskbar_update = Instant::now();
    let mut last_taskbar_pct: i32 = -1;

    loop {
        state.drain_device_events(&handle);
        state.drain_hash_job();
        state.drain_import_job();
        state.drain_prepare();
        state.drain_write();
        state.drain_single_retries();
        state.auto_transition_finished();

        terminal.draw(|f| ui::render(f, &state))?;

        let new_title = compute_window_title(&state);
        if new_title != last_title {
            let mut out = io::stdout();
            let _ = execute!(out, SetTitle(&new_title));
            last_title = new_title;
        }

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
    }

    taskbar.clear();

    Ok(state)
}

fn set_window_title(state: &AppState) -> io::Result<()> {
    let title = compute_window_title(state);
    let mut out = io::stdout();
    execute!(out, SetTitle(&title))?;
    Ok(())
}

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
        Screen::FilePicker => handle_key_file_picker(state, key),
    }
}

fn handle_key_main(state: &mut AppState, key: KeyEvent) -> bool {
    // Если находимся в состоянии Завершено, нажатие Enter или Space также сбрасывает в Ready/Idle
    if state.status == AppStatus::Finished {
        if matches!(key.code, KeyCode::Enter | KeyCode::Char(' ')) {
            state.reset_to_idle();
            return false;
        }
    }

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
        's' => state.start_write_flow(),
        'r' => {
            // Кнопка R переключает в состояние Idle / Ready для следующей партии флешек
            state.reset_to_idle();
        }
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
            'a' => state.open_add_file_dialog(),
            'b' => state.open_builtin_file_picker(mass_usb_flasher::app::FilePickerMode::AddImage),
            'o' => state.open_ephemeral_file_dialog(),
            'd' => state.delete_selected_from_catalog(),
            'r' => state.start_rename_selected(),
            'f' => state.open_import_folder_dialog(),
            _ => {}
        },
        _ => {}
    }
    false
}

fn handle_key_file_picker(state: &mut AppState, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Up => {
            if state.file_picker_selected > 0 {
                state.file_picker_selected -= 1;
            }
        }
        KeyCode::Down => {
            if !state.file_picker_entries.is_empty()
                && state.file_picker_selected + 1 < state.file_picker_entries.len()
            {
                state.file_picker_selected += 1;
            }
        }
        KeyCode::PageUp => {
            state.file_picker_selected = state.file_picker_selected.saturating_sub(10);
        }
        KeyCode::PageDown => {
            if !state.file_picker_entries.is_empty() {
                state.file_picker_selected = (state.file_picker_selected + 10)
                    .min(state.file_picker_entries.len().saturating_sub(1));
            }
        }
        KeyCode::Backspace => {
            if let Some(parent) = state.file_picker_dir.parent().map(|p| p.to_path_buf()) {
                state.file_picker_dir = parent;
                state.file_picker_selected = 0;
                state.refresh_file_picker_entries();
            }
        }
        KeyCode::Enter => {
            if let Some(item) = state
                .file_picker_entries
                .get(state.file_picker_selected)
                .cloned()
            {
                if item.is_dir {
                    state.file_picker_dir = item.path;
                    state.file_picker_selected = 0;
                    state.refresh_file_picker_entries();
                } else {
                    match state.file_picker_mode {
                        mass_usb_flasher::app::FilePickerMode::AddImage => {
                            state.apply_add_image_path(item.path);
                        }
                        mass_usb_flasher::app::FilePickerMode::OpenOnce => {
                            state.apply_ephemeral_image_path(item.path);
                        }
                        mass_usb_flasher::app::FilePickerMode::ImportFolder => {}
                    }
                }
            }
        }
        KeyCode::Char(' ')
            if state.file_picker_mode == mass_usb_flasher::app::FilePickerMode::ImportFolder =>
        {
            let dir = state.file_picker_dir.clone();
            state.apply_import_folder(dir);
        }
        KeyCode::Esc => {
            state.screen = Screen::ImageSelect;
        }
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
        KeyCode::Enter => state.commit_new_image(),
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
        KeyCode::Enter => state.commit_rename(),
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
