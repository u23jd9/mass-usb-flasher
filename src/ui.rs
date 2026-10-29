use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table},
    Frame,
};

use crate::app::{AppState, AppStatus, Screen, SlotStatus};
use crate::catalog::ImportProgress;
use crate::i18n::{Lang, T, VERSION, WATERMARK};

// --- Стили ---

fn dim() -> Style { Style::default().fg(Color::DarkGray) }
fn gray() -> Style { Style::default().fg(Color::Gray) }
fn cyan() -> Style { Style::default().fg(Color::Cyan) }
fn green() -> Style { Style::default().fg(Color::Green) }
fn yellow() -> Style { Style::default().fg(Color::Yellow) }
fn magenta() -> Style { Style::default().fg(Color::Magenta) }
fn red() -> Style { Style::default().fg(Color::Red) }
fn bold(s: Style) -> Style { s.add_modifier(Modifier::BOLD) }
fn italic(s: Style) -> Style { s.add_modifier(Modifier::ITALIC) }
fn label(text: &'static str) -> Span<'static> { Span::styled(text, dim()) }

pub fn render(f: &mut Frame, state: &AppState) {
    render_main(f, state);
    match state.screen {
        Screen::Main => {}
        Screen::ImageSelect => render_image_select(f, state),
        Screen::AddImageName => {
            render_image_select(f, state);
            render_input_modal(f, state, state.t().title_new_image, state.t().prompt_work_name);
        }
        Screen::RenameImage => {
            render_image_select(f, state);
            render_input_modal(f, state, state.t().title_rename, state.t().prompt_new_name);
        }
        Screen::Help => render_help(f, state),
        Screen::FilePicker => render_file_picker(f, state),
    }
}

fn render_main(f: &mut Frame, state: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(6),
            Constraint::Min(5),
            Constraint::Length(5),
        ])
        .split(f.area());

    render_header(f, state, chunks[0]);
    render_devices(f, state, chunks[1]);
    render_footer(f, state, chunks[2]);
}

fn render_header(f: &mut Frame, state: &AppState, area: Rect) {
    let t = state.t();

    let (status_str, status_style) = match state.status {
        AppStatus::Idle => (t.status_idle, dim()),
        AppStatus::WaitingForDevices => (t.status_waiting, yellow()),
        AppStatus::Ready => (t.status_ready, green()),
        AppStatus::Running => (t.status_running, cyan()),
        AppStatus::Finished => (t.status_finished, magenta()),
    };

    let active_count = state.active_or_ready_count();
    let total_found = state.devices.len();
    let target = state.target_count;

    let target_style = if active_count >= target { green() } else { yellow() };

    let target_text = if total_found > target && state.status != AppStatus::Running {
        format!("{}/{}", active_count, target)
            + &t.target_extras_fmt.replace("{}", &total_found.to_string())
    } else {
        format!("{}/{}", active_count, target)
    };

    let line1 = Line::from(vec![
        label(t.lbl_status),
        Span::raw(" "),
        Span::styled(format!("{:<20}", status_str), bold(status_style)),
        label(t.lbl_target),
        Span::raw(" "),
        Span::styled(target_text, target_style),
        Span::styled(" [+/-]", dim()),
        label(t.lbl_min),
        Span::raw(format!(" {:.0} GB", state.min_size_bytes as f64 / 1e9)),
        label(t.lbl_max),
        Span::raw(format!(" {:.0} GB", state.max_size_bytes as f64 / 1e9)),
    ]);

    let line2 = render_image_line(state, area);
    let line3 = render_hash_line(state);
    let line4 = render_timing_line(state);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {} ", t.title_app));
    f.render_widget(
        Paragraph::new(vec![line1, line2, line3, line4]).block(block),
        area,
    );
}

fn render_image_line(state: &AppState, area: Rect) -> Line<'static> {
    let t = state.t();
    let Some(p) = &state.image_path else {
        return Line::from(vec![
            label(t.lbl_image),
            Span::raw(" "),
            Span::styled(t.image_none, dim()),
        ]);
    };

    let work = state
        .image_work_name
        .clone()
        .unwrap_or_else(|| t.image_no_name.to_string());

    let s = p.display().to_string();
    let width = area.width.saturating_sub(12) as usize;
    let short = truncate_left(&s, width);
    let size_str = if state.image_size > 0 {
        format!("  ({:.2} GB)", state.image_size as f64 / 1e9)
    } else {
        String::new()
    };

    Line::from(vec![
        label(t.lbl_image),
        Span::raw(" "),
        Span::styled(format!("[{}] ", work), bold(magenta())),
        Span::raw(short),
        Span::styled(size_str, gray()),
    ])
}

fn render_hash_line(state: &AppState) -> Line<'static> {
    let t = state.t();
    match (&state.image_sha256, state.image_hash_progress) {
        (Some(h), _) => Line::from(vec![
            label(t.lbl_sha256),
            Span::raw(" "),
            Span::styled(h.clone(), green()),
        ]),
        (None, Some((done, total))) => {
            let pct = if total > 0 {
                (done as f64 / total as f64) * 100.0
            } else {
                0.0
            };
            Line::from(vec![
                label(t.lbl_sha256),
                Span::raw(" "),
                Span::styled(
                    format!(
                        "{} {:5.1}% ({:.0}/{:.0} MB)",
                        t.sha_computing,
                        pct,
                        done as f64 / 1_048_576.0,
                        total as f64 / 1_048_576.0
                    ),
                    italic(yellow()),
                ),
            ])
        }
        (None, None) => {
            let text = if state.image_path.is_some() {
                t.sha_starting
            } else {
                "—"
            };
            Line::from(vec![
                label(t.lbl_sha256),
                Span::raw(" "),
                Span::styled(text, dim()),
            ])
        }
    }
}

fn render_timing_line(state: &AppState) -> Line<'static> {
    let t = state.t();
    if state.status == AppStatus::Finished {
        return Line::from(vec![
            Span::styled(format!("★ {} ★", t.status_finished_hint), bold(magenta())),
        ]);
    }
    let Some(elapsed) = state.elapsed_secs() else {
        return Line::from("");
    };

    let avg_bytes = state.average_bytes_written();
    let total = state.write_total_bytes;
    let written = format!(
        "{:.2} / {:.2} GB",
        avg_bytes as f64 / 1e9,
        total as f64 / 1e9
    );

    let speed = state
        .per_device_speed_mbps()
        .map(|s| format!("{:.1} {}", s, t.speed_per_device))
        .unwrap_or_else(|| "—".to_string());

    let text = format!(
        "{} {}     {} {}     {} {}",
        t.lbl_elapsed,
        format_elapsed(elapsed),
        t.lbl_speed,
        speed,
        t.lbl_written,
        written,
    );
    Line::from(Span::styled(text, cyan()))
}

fn render_devices(f: &mut Frame, state: &AppState, area: Rect) {
    let t = state.t();

    let header_cells = [
        t.th_id, t.th_path, t.th_size, t.th_product, t.th_serial, t.th_status, t.th_progress,
    ]
    .into_iter()
    .map(|h| Cell::from(h).style(bold(Style::default())));
    let header = Row::new(header_cells).height(1).bottom_margin(1);

    let rows: Vec<Row> = state
        .devices
        .iter()
        .map(|(n, slot)| {
            let gb = slot.info.size_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
            let (status_str, status_style) = slot_status_display(state, slot.status);
            let progress = slot_progress_display(slot);

            Row::new(vec![
                Cell::from(n.to_string()),
                Cell::from(slot.info.path.clone()),
                Cell::from(format!("{:.2}", gb)),
                Cell::from(truncate(&slot.info.product, 22)),
                Cell::from(truncate(&slot.info.serial, 16)),
                Cell::from(status_str).style(status_style),
                Cell::from(progress).style(status_style),
            ])
        })
        .collect();

    let rows = if rows.is_empty() {
        vec![Row::new(vec![Cell::from(empty_devices_text(state)).style(dim())])]
    } else {
        rows
    };

    let title = t
        .title_devices_fmt
        .replace("{}", &state.devices.len().to_string())
        .replacen("{}", &state.ready_count().to_string(), 1)
        .replacen("{}", &state.target_count.to_string(), 1);

    let table = Table::new(
        rows,
        [
            Constraint::Length(4),
            Constraint::Length(20),
            Constraint::Length(10),
            Constraint::Length(24),
            Constraint::Length(18),
            Constraint::Length(11),
            Constraint::Min(10),
        ],
    )
    .header(header)
    .block(Block::default().borders(Borders::ALL).title(title));

    f.render_widget(table, area);
}

fn empty_devices_text(state: &AppState) -> &'static str {
    match state.lang {
        Lang::En => "No USB devices connected. Plug in drives and wait a few seconds.",
        Lang::Ru => "Нет подключённых USB-устройств. Подключи флешки и подожди пару секунд.",
    }
}

fn slot_status_display(state: &AppState, s: SlotStatus) -> (&'static str, Style) {
    let t = state.t();
    match s {
        SlotStatus::Waiting => (t.slot_waiting, dim()),
        SlotStatus::Ready => (t.slot_ready, green()),
        SlotStatus::Writing => (t.slot_writing, yellow()),
        SlotStatus::Verifying => (t.slot_verifying, yellow()),
        SlotStatus::Formatting => (t.slot_formatting, yellow()),
        SlotStatus::Done => (t.slot_done, bold(green())),
        SlotStatus::Failed => (t.slot_failed, bold(red())),
    }
}

fn slot_progress_display(slot: &crate::app::DeviceSlot) -> String {
    match slot.status {
        SlotStatus::Failed => {
            if slot.message.is_empty() {
                "FAIL".to_string()
            } else {
                truncate(&slot.message, 30)
            }
        }
        SlotStatus::Done => "100.0%".to_string(),
        SlotStatus::Writing | SlotStatus::Verifying | SlotStatus::Formatting => {
            format!("{:>5.1}%", slot.progress * 100.0)
        }
        _ => String::new(),
    }
}

fn render_footer(f: &mut Frame, state: &AppState, area: Rect) {
    let t = state.t();

    let mut keys = Vec::new();
    if state.status == AppStatus::Finished {
        keys.push(Span::styled("[R/Enter]", bold(green())));
        keys.push(Span::styled(format!(" {}  ", t.fk_reset), bold(green())));
    }
    keys.push(Span::styled("[I]", yellow()));
    keys.push(Span::raw(format!(" {}  ", t.fk_open)));
    keys.push(Span::styled("[S]", yellow()));
    keys.push(Span::raw(format!(" {}  ", t.fk_start)));
    keys.push(Span::styled("[H]", yellow()));
    keys.push(Span::raw(format!(" {}  ", t.fk_help)));
    keys.push(Span::styled("[+/-]", yellow()));
    keys.push(Span::raw(format!(" {}  ", t.fk_target)));
    keys.push(Span::styled("[L]", yellow()));
    keys.push(Span::raw(format!(" {}  ", t.fk_lang)));
    keys.push(Span::styled("[Q]", yellow()));
    keys.push(Span::raw(format!(" {}", t.fk_quit)));

    let hotkeys = Line::from(keys);

    let second = if let Some(p) = &state.import_progress {
        render_import_status(state, p)
    } else {
        Line::from(Span::styled(
            state.log.last().cloned().unwrap_or_else(|| "—".to_string()),
            dim(),
        ))
    };

    let watermark = format!("{}  ·  v{}", WATERMARK, VERSION);
    let inner_width = area.width.saturating_sub(2) as usize;
    let wm_len = watermark.chars().count();
    let pad = inner_width.saturating_sub(wm_len);
    let third = Line::from(Span::styled(
        format!("{}{}", " ".repeat(pad), watermark),
        dim(),
    ));

    let block = Block::default().borders(Borders::ALL);
    f.render_widget(
        Paragraph::new(vec![hotkeys, second, third]).block(block),
        area,
    );
}

fn render_import_status(state: &AppState, p: &ImportProgress) -> Line<'static> {
    let t = state.t();
    let tag = Span::styled("[import] ", yellow());
    match p {
        ImportProgress::Scanning { total } => Line::from(vec![
            tag,
            Span::raw(t.imp_scanning_fmt.replace("{}", &total.to_string())),
        ]),
        ImportProgress::HashingFile {
            current,
            total,
            name,
            bytes_done,
            bytes_total,
        } => {
            let pct = if *bytes_total > 0 {
                (*bytes_done as f64 / *bytes_total as f64) * 100.0
            } else {
                0.0
            };
            let prefix = t
                .imp_hashing_fmt
                .replace("{}", &current.to_string())
                .replacen("{}", &total.to_string(), 1);
            Line::from(vec![
                tag,
                Span::raw(format!("{} ", prefix)),
                Span::styled(name.clone(), cyan()),
                Span::raw(format!(
                    " — {:.1}% ({:.0}/{:.0} MB)",
                    pct,
                    *bytes_done as f64 / 1_048_576.0,
                    *bytes_total as f64 / 1_048_576.0
                )),
            ])
        }
        ImportProgress::FileDone { name } => Line::from(vec![
            tag,
            Span::raw(format!("{} {}", t.imp_ready, name)),
        ]),
        ImportProgress::Done { imported, .. } => Line::from(vec![
            tag,
            Span::raw(t.imp_done_fmt.replace("{}", &imported.to_string())),
        ]),
        ImportProgress::Error(e) => Line::from(vec![
            Span::styled("[import] ", red()),
            Span::raw(t.imp_error_fmt.replace("{}", e)),
        ]),
    }
}

fn render_image_select(f: &mut Frame, state: &AppState) {
    let t = state.t();
    let area = centered_rect(85, 70, f.area());
    f.render_widget(Clear, area);

    let header = Row::new(vec![
        Cell::from("Work Name").style(bold(Style::default())),
        Cell::from("Size").style(bold(Style::default())),
        Cell::from("SHA256").style(bold(Style::default())),
        Cell::from("Path").style(bold(Style::default())),
    ])
    .height(1)
    .bottom_margin(1);

    let rows: Vec<Row> = state
        .catalog
        .images
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let short_hash = if e.sha256.len() >= 12 {
                format!("{}…", &e.sha256[..12])
            } else {
                e.sha256.clone()
            };
            let style = if i == state.image_select_index {
                Style::default()
                    .bg(Color::DarkGray)
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };

            Row::new(vec![
                Cell::from(e.work_name.clone()),
                Cell::from(format!("{:.2} GB", e.size_bytes as f64 / 1e9)),
                Cell::from(short_hash),
                Cell::from(truncate(&e.path.display().to_string(), 45)),
            ])
            .style(style)
        })
        .collect();

    let rows = if rows.is_empty() {
        vec![Row::new(vec![Cell::from(t.catalog_empty).style(dim())])]
    } else {
        rows
    };

    let title = t
        .title_catalog_fmt
        .replace("{}", &state.catalog.images.len().to_string());

    let table = Table::new(
        rows,
        [
            Constraint::Length(20),
            Constraint::Length(10),
            Constraint::Length(14),
            Constraint::Min(30),
        ],
    )
    .header(header)
    .block(Block::default().borders(Borders::ALL).title(title));

    f.render_widget(table, area);

    if area.height > 3 {
        let hint_area = Rect {
            x: area.x,
            y: area.y + area.height - 3,
            width: area.width,
            height: 2,
        };
        let hint = Paragraph::new(vec![Line::from(vec![
            Span::styled("[Enter]", yellow()),
            Span::raw(format!(" {}  ", t.cat_hint_enter)),
            Span::styled("[A]", yellow()),
            Span::raw(format!(" {}  ", t.cat_hint_a)),
            Span::styled("[O]", yellow()),
            Span::raw(format!(" {}  ", t.cat_hint_o)),
            Span::styled("[F]", yellow()),
            Span::raw(format!(" {}  ", t.cat_hint_f)),
            Span::styled("[R]", yellow()),
            Span::raw(format!(" {}  ", t.cat_hint_r)),
            Span::styled("[D]", yellow()),
            Span::raw(format!(" {}  ", t.cat_hint_d)),
            Span::styled("[Esc]", yellow()),
            Span::raw(format!(" {}", t.cat_hint_esc)),
        ])]);
        f.render_widget(hint, hint_area);
    }
}

fn render_input_modal(f: &mut Frame, state: &AppState, title: &str, prompt: &str) {
    let area = centered_rect(60, 25, f.area());
    f.render_widget(Clear, area);

    let t = state.t();
    let path_str = state
        .pending_image_path
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_default();

    let mut text = Vec::new();
    if !path_str.is_empty() {
        text.push(Line::from(Span::styled(truncate(&path_str, 80), dim())));
        text.push(Line::from(format!(
            "{} {:.2} GB",
            t.modal_size,
            state.pending_image_size as f64 / 1e9
        )));
        text.push(Line::from(""));
    }
    text.push(Line::from(Span::styled(
        format!("{} ({})", prompt, t.prompt_hint),
        gray(),
    )));
    text.push(Line::from(Span::styled(
        format!("> {}_", state.input_buffer),
        bold(cyan()),
    )));

    let block = Block::default().borders(Borders::ALL).title(title.to_string());
    f.render_widget(Paragraph::new(text).block(block), area);
}

fn render_help(f: &mut Frame, state: &AppState) {
    let area = centered_rect(80, 85, f.area());
    f.render_widget(Clear, area);

    let catalog_path = state.catalog_path.display().to_string();
    let t: &'static T = state.t();

    let text = match state.lang {
        Lang::En => build_help_en(t, &catalog_path),
        Lang::Ru => build_help_ru(t, &catalog_path),
    };

    let block = Block::default().borders(Borders::ALL).title(t.title_help);
    f.render_widget(Paragraph::new(text).block(block), area);
}

fn build_help_en(_t: &T, catalog_path: &str) -> Vec<Line<'static>> {
    vec![
        Line::from(Span::styled("Main screen", bold(yellow()))),
        Line::from("  I        Open image catalog"),
        Line::from("  S        Start writing"),
        Line::from("  R        Reset to Ready/Idle for next batch (after write)"),
        Line::from("  + / -    Change target device count"),
        Line::from("  H        This help"),
        Line::from("  L        Toggle language (EN / RU)"),
        Line::from("  Q        Quit"),
        Line::from("  Ctrl+C   Quit (always)"),
        Line::from(""),
        Line::from(Span::styled("Image catalog", bold(yellow()))),
        Line::from("  Up/Down  Navigate"),
        Line::from("  Enter    Select image (SHA-256 already known)"),
        Line::from("  A        Add image from file (with work name)"),
        Line::from("  O        Open image once, without saving to catalog"),
        Line::from("  F        Import entire folder (recursive)"),
        Line::from("  R        Rename selected image"),
        Line::from("  D        Delete image from catalog"),
        Line::from("  Esc      Back"),
        Line::from(""),
        Line::from(Span::styled("Text input / rename", bold(yellow()))),
        Line::from("  Enter      Confirm"),
        Line::from("  Esc        Cancel"),
        Line::from("  Backspace  Delete character"),
        Line::from(""),
        Line::from(Span::styled("Data storage", bold(yellow()))),
        Line::from(vec![
            Span::raw("  Global catalog: "),
            Span::styled(catalog_path.to_string(), cyan()),
        ]),
        Line::from("  Folder manifest: .muf-images.json in the root of the images folder"),
        Line::from("  Manifest paths are relative, separator `/` — cross-platform"),
        Line::from(""),
        Line::from(Span::styled("  [any key] — close help", dim())),
    ]
}

fn build_help_ru(_t: &T, catalog_path: &str) -> Vec<Line<'static>> {
    vec![
        Line::from(Span::styled("Главный экран", bold(yellow()))),
        Line::from("  I        Открыть каталог образов"),
        Line::from("  S        Начать запись"),
        Line::from("  R        Сброс в Готово/Ожидание для новой партии (после записи)"),
        Line::from("  + / -    Изменить количество целевых устройств"),
        Line::from("  H        Эта справка"),
        Line::from("  L        Переключить язык (EN / RU)"),
        Line::from("  Q        Выход"),
        Line::from("  Ctrl+C   Выход (всегда)"),
        Line::from(""),
        Line::from(Span::styled("Каталог образов", bold(yellow()))),
        Line::from("  Up/Down  Навигация"),
        Line::from("  Enter    Выбрать образ (SHA-256 уже известен)"),
        Line::from("  A        Добавить образ из файла (с work name)"),
        Line::from("  O        Открыть образ разово, без сохранения в каталог"),
        Line::from("  F        Импортировать всю папку (рекурсивно)"),
        Line::from("  R        Переименовать выбранный образ"),
        Line::from("  D        Удалить образ из каталога"),
        Line::from("  Esc      Назад"),
        Line::from(""),
        Line::from(Span::styled("Ввод имени / переименование", bold(yellow()))),
        Line::from("  Enter      Подтвердить"),
        Line::from("  Esc        Отмена"),
        Line::from("  Backspace  Удалить символ"),
        Line::from(""),
        Line::from(Span::styled("Хранение данных", bold(yellow()))),
        Line::from(vec![
            Span::raw("  Глобальный каталог: "),
            Span::styled(catalog_path.to_string(), cyan()),
        ]),
        Line::from("  Папочный манифест: .muf-images.json в корне папки с образами"),
        Line::from("  Пути в манифесте относительные, разделитель `/` — кроссплатформенно"),
        Line::from(""),
        Line::from(Span::styled("  [любая клавиша] — закрыть справку", dim())),
    ]
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup[1])[1]
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let t: String = s.chars().take(n.saturating_sub(1)).collect();
        format!("{}…", t)
    }
}

fn truncate_left(s: &str, n: usize) -> String {
    let count = s.chars().count();
    if count <= n {
        return s.to_string();
    }
    if n <= 1 {
        return "…".to_string();
    }
    let skip = count - (n - 1);
    let tail: String = s.chars().skip(skip).collect();
    format!("…{}", tail)
}

fn format_elapsed(secs: f64) -> String {
    let s = secs as u64;
    if s < 60 {
        format!("{}s", s)
    } else if s < 3600 {
        format!("{}m {:02}s", s / 60, s % 60)
    } else {
        format!("{}h {:02}m {:02}s", s / 3600, (s % 3600) / 60, s % 60)
    }
}

fn render_file_picker(f: &mut Frame, state: &AppState) {
    let area = centered_rect(80, 80, f.area());
    f.render_widget(Clear, area);

    let t = state.t();
    let title = match state.file_picker_mode {
        crate::app::FilePickerMode::AddImage => format!(" {} [Add] ", t.gui_btn_select),
        crate::app::FilePickerMode::OpenOnce => format!(" {} [Open] ", t.gui_btn_open_once),
        crate::app::FilePickerMode::ImportFolder => format!(" {} [Folder] ", t.cat_hint_f),
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(3),
        ])
        .split(area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(bold(cyan()))
        .title(title);
    f.render_widget(block, area);

    let path_str = state.file_picker_dir.display().to_string();
    let path_p = Paragraph::new(Line::from(vec![
        Span::styled(" Directory: ", bold(yellow())),
        Span::styled(truncate(&path_str, 70), gray()),
    ])).block(Block::default().borders(Borders::BOTTOM));
    f.render_widget(path_p, chunks[0]);

    let rows: Vec<Row> = state.file_picker_entries.iter().enumerate().map(|(idx, item)| {
        let is_sel = idx == state.file_picker_selected;
        let prefix = if item.is_dir { "📁 " } else { "📄 " };
        let name_style = if is_sel {
            bold(green())
        } else if item.is_dir {
            bold(cyan())
        } else {
            gray()
        };

        let size_str = if item.is_dir {
            "—".to_string()
        } else {
            format!("{:.2} GB", item.size as f64 / 1e9)
        };

        let cells = vec![
            Cell::from(format!("{}{}", prefix, item.name)).style(name_style),
            Cell::from(size_str).style(dim()),
        ];

        let mut row = Row::new(cells);
        if is_sel {
            row = row.style(Style::default().bg(Color::Rgb(40, 50, 70)));
        }
        row
    }).collect();

    let widths = [Constraint::Min(30), Constraint::Length(14)];
    let table = Table::new(rows, widths)
        .block(Block::default().borders(Borders::NONE));
    f.render_widget(table, chunks[1]);

    let hint = if state.file_picker_mode == crate::app::FilePickerMode::ImportFolder {
        "[Enter/Space] Select Folder  ·  [↑/↓] Navigate  ·  [Backspace] Up  ·  [Esc] Cancel"
    } else {
        "[Enter] Select File / Enter Dir  ·  [↑/↓] Navigate  ·  [Backspace] Up  ·  [Esc] Cancel"
    };
    let footer_p = Paragraph::new(Line::from(vec![
        Span::styled(hint, dim()),
    ])).block(Block::default().borders(Borders::TOP));
    f.render_widget(footer_p, chunks[2]);
}
