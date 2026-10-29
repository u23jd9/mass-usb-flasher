#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![allow(dead_code)]

use std::time::{Duration, Instant};

use eframe::egui::{self, Align, Color32, Layout, RichText, Rounding, Stroke, Vec2};

use mass_usb_flasher::app::{AppState, AppStatus, SlotStatus};
use mass_usb_flasher::catalog::Catalog;
use mass_usb_flasher::common::{
    compute_window_title, ensure_automount_enabled, ensure_elevated, format_elapsed,
    parse_args, parse_target_count_arg,
};
use mass_usb_flasher::device::watcher::{DeviceFilter, WatcherConfig, WatcherHandle};
use mass_usb_flasher::i18n::{VERSION, WATERMARK, WATERMARK_URL};
use mass_usb_flasher::taskbar::Taskbar;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuiTheme {
    Dark,
    Light,
}

struct ThemeColors {
    top_panel_bg: Color32,
    bottom_panel_bg: Color32,
    card_bg: Color32,
    card_stroke: Color32,
    text_primary: Color32,
    text_muted: Color32,
    empty_box_bg: Color32,
    empty_icon_color: Color32,
}

impl GuiTheme {
    pub fn visuals(self) -> egui::Visuals {
        match self {
            GuiTheme::Dark => {
                let mut v = egui::Visuals::dark();
                v.window_rounding = Rounding::same(8.0);
                v.panel_fill = Color32::from_rgb(22, 23, 28);
                v.window_fill = Color32::from_rgb(28, 30, 36);
                v
            }
            GuiTheme::Light => {
                let mut v = egui::Visuals::light();
                v.window_rounding = Rounding::same(8.0);
                v.panel_fill = Color32::from_rgb(246, 248, 252);
                v.window_fill = Color32::from_rgb(255, 255, 255);
                v
            }
        }
    }

    fn colors(self) -> ThemeColors {
        match self {
            GuiTheme::Dark => ThemeColors {
                top_panel_bg: Color32::from_rgb(28, 30, 36),
                bottom_panel_bg: Color32::from_rgb(20, 21, 26),
                card_bg: Color32::from_rgb(34, 36, 44),
                card_stroke: Color32::from_rgb(52, 56, 68),
                text_primary: Color32::from_rgb(235, 238, 245),
                text_muted: Color32::from_rgb(145, 150, 165),
                empty_box_bg: Color32::from_rgb(28, 29, 36),
                empty_icon_color: Color32::from_rgb(120, 120, 140),
            },
            GuiTheme::Light => ThemeColors {
                top_panel_bg: Color32::from_rgb(238, 241, 248),
                bottom_panel_bg: Color32::from_rgb(230, 234, 242),
                card_bg: Color32::from_rgb(255, 255, 255),
                card_stroke: Color32::from_rgb(215, 220, 230),
                text_primary: Color32::from_rgb(25, 28, 36),
                text_muted: Color32::from_rgb(105, 112, 128),
                empty_box_bg: Color32::from_rgb(240, 242, 248),
                empty_icon_color: Color32::from_rgb(160, 165, 185),
            },
        }
    }
}

fn main() -> eframe::Result<()> {
    if !ensure_elevated(true) {
        std::process::exit(1);
    }

    let _ = ensure_automount_enabled();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Mass USB Flasher")
            .with_inner_size([1040.0, 720.0])
            .with_min_inner_size([880.0, 540.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Mass USB Flasher",
        options,
        Box::new(|cc| Ok(Box::new(MufGuiApp::new(cc)))),
    )
}

struct MufGuiApp {
    state: AppState,
    watcher: WatcherHandle,
    taskbar: Taskbar,
    theme: GuiTheme,
    show_catalog_modal: bool,
    show_help_modal: bool,
    show_file_picker_modal: bool,
    show_log_panel: bool,
    log_autoscroll: bool,
    last_taskbar_update: Instant,
    last_taskbar_pct: i32,
}

impl MufGuiApp {
    fn new(cc: &eframe::CreationContext) -> Self {
        let theme = GuiTheme::Dark;
        cc.egui_ctx.set_visuals(theme.visuals());

        let mut state = AppState::new();

        if let Some(n) = parse_target_count_arg() {
            state.set_target_count(n);
        }

        state.catalog = Catalog::load_or_default(&state.catalog_path);
        state.push_log(format!(
            "Catalog loaded: {} entries",
            state.catalog.images.len()
        ));

        let filter = DeviceFilter {
            usb_only: true,
            min_size_bytes: 4 * 1024 * 1024 * 1024,
            max_size_bytes: 128 * 1024 * 1024 * 1024,
        };
        let watcher = mass_usb_flasher::device::watcher::spawn(WatcherConfig {
            poll_interval: Duration::from_millis(500),
            max_disks: 16,
            filter,
        });

        if let Some(path) = parse_args() {
            state.load_image_from_cli(path);
        }

        Self {
            state,
            watcher,
            taskbar: Taskbar::new(),
            theme,
            show_catalog_modal: false,
            show_help_modal: false,
            show_file_picker_modal: false,
            show_log_panel: false,
            log_autoscroll: true,
            last_taskbar_update: Instant::now(),
            last_taskbar_pct: -1,
        }
    }

    fn update_taskbar(&mut self) {
        if self.last_taskbar_update.elapsed() >= Duration::from_millis(100) {
            let total = self.state.write_total_bytes;
            let done = self.state.slowest_bytes_written();
            if total > 0 && matches!(self.state.status, AppStatus::Running) {
                let pct = ((done as f64 / total as f64) * 100.0)
                    .clamp(0.0, 100.0)
                    .round() as i32;
                if pct != self.last_taskbar_pct {
                    self.taskbar.set(done, total);
                    self.last_taskbar_pct = pct;
                }
            } else if self.last_taskbar_pct >= 0 {
                self.taskbar.clear();
                self.last_taskbar_pct = -1;
            }
            self.last_taskbar_update = Instant::now();
        }
    }
}

impl eframe::App for MufGuiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Фоновая обработка очередей
        self.state.drain_device_events(&self.watcher);
        self.state.drain_hash_job();
        self.state.drain_import_job();
        self.state.drain_prepare();
        self.state.drain_write();
        self.state.drain_single_retries();
        self.state.auto_transition_finished();
        self.update_taskbar();

        // Задаём заголовок окна
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(compute_window_title(
            &self.state,
        )));

        // Во время записи или хеширования перерисовываем чаще
        if matches!(self.state.status, AppStatus::Running)
            || self.state.image_hash_progress.is_some()
            || self.state.import_job.is_some()
            || self.state.has_active_retries()
        {
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        let colors = self.theme.colors();

        // Handle drag-and-drop of disk images
        ctx.input(|i| {
            if !i.raw.dropped_files.is_empty() {
                for file in &i.raw.dropped_files {
                    if let Some(path) = &file.path {
                        self.state.select_image_path(path.clone(), true);
                        break;
                    }
                }
            }
        });

        // --- ВЕРХНЯЯ ПАНЕЛЬ (Header & Image Controls) ---
        egui::TopBottomPanel::top("top_panel")
            .frame(
                egui::Frame::none()
                    .fill(colors.top_panel_bg)
                    .inner_margin(12.0),
            )
            .show(ctx, |ui| {
                self.render_top_bar(ui);
                ui.add_space(8.0);
                self.render_image_card(ui);
            });

        // --- НИЖНЯЯ ПАНЕЛЬ ДЕЙСТВИЙ (Start, Reset, Help, Status) ---
        egui::TopBottomPanel::bottom("bottom_panel")
            .frame(
                egui::Frame::none()
                    .fill(colors.bottom_panel_bg)
                    .inner_margin(12.0),
            )
            .show(ctx, |ui| {
                self.render_bottom_bar(ui);
            });

        // --- ПАНЕЛЬ ЛОГА (Activity Log: docked above bottom bar, resizable) ---
        if self.show_log_panel {
            egui::TopBottomPanel::bottom("log_panel")
                .resizable(true)
                .default_height(160.0)
                .min_height(80.0)
                .max_height(360.0)
                .frame(
                    egui::Frame::none()
                        .fill(colors.card_bg)
                        .stroke(Stroke::new(1.0_f32, colors.card_stroke))
                        .inner_margin(8.0),
                )
                .show(ctx, |ui| {
                    self.render_log_viewer(ui);
                });
        }

        // --- ЦЕНТРАЛЬНАЯ ПАНЕЛЬ (Таблица устройств) ---
        egui::CentralPanel::default().show(ctx, |ui| {
            self.render_devices_table(ui);
        });

        // --- МОДАЛЬНЫЕ ОКНА ---
        if self.show_catalog_modal {
            self.render_catalog_window(ctx);
        }

        if self.show_help_modal {
            self.render_help_window(ctx);
        }

        if self.show_file_picker_modal {
            self.render_file_picker_window(ctx);
        }
    }
}

impl MufGuiApp {
    fn render_top_bar(&mut self, ui: &mut egui::Ui) {
        let t = self.state.t();
        ui.horizontal(|ui| {
            ui.heading(
                RichText::new("⚡ Mass USB Flasher")
                    .strong()
                    .color(Color32::from_rgb(100, 200, 255)),
            );
            ui.label(RichText::new(format!("v{}", VERSION)).weak());

            ui.add_space(16.0);

            // Статус-бейдж
            let (status_text, bg_color, fg_color) = match self.state.status {
                AppStatus::Idle => (t.status_idle, Color32::from_rgb(60, 60, 70), Color32::WHITE),
                AppStatus::WaitingForDevices => (
                    t.status_waiting,
                    Color32::from_rgb(180, 110, 0),
                    Color32::WHITE,
                ),
                AppStatus::Ready => (
                    t.status_ready,
                    Color32::from_rgb(34, 139, 34),
                    Color32::WHITE,
                ),
                AppStatus::Running => (
                    t.status_running,
                    Color32::from_rgb(25, 118, 210),
                    Color32::WHITE,
                ),
                AppStatus::Finished => (
                    t.status_finished,
                    Color32::from_rgb(142, 36, 170),
                    Color32::WHITE,
                ),
            };

            egui::Frame::none()
                .fill(bg_color)
                .rounding(Rounding::same(4.0))
                .inner_margin(egui::Margin::symmetric(10.0, 4.0))
                .show(ui, |ui| {
                    ui.label(RichText::new(status_text).strong().color(fg_color));
                });

            // Управление целевым количеством
            ui.add_space(20.0);
            ui.label(RichText::new(t.lbl_target).strong());
            if ui.button("➖").clicked() {
                self.state.decrease_target();
            }
            ui.label(
                RichText::new(format!("{}", self.state.target_count))
                    .strong()
                    .size(16.0)
                    .color(Color32::from_rgb(255, 215, 0)),
            );
            if ui.button("➕").clicked() {
                self.state.increase_target();
            }

            // Переключатели темы и языка
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .button(
                        RichText::new(format!("🌐 {}", self.state.lang.code()))
                            .color(Color32::from_rgb(100, 180, 255)),
                    )
                    .clicked()
                {
                    self.state.lang = self.state.lang.toggle();
                }

                let theme_text = match self.theme {
                    GuiTheme::Dark => t.gui_theme_dark,
                    GuiTheme::Light => t.gui_theme_light,
                };
                if ui.button(RichText::new(theme_text)).clicked() {
                    self.theme = match self.theme {
                        GuiTheme::Dark => GuiTheme::Light,
                        GuiTheme::Light => GuiTheme::Dark,
                    };
                    ui.ctx().set_visuals(self.theme.visuals());
                }
            });
        });
    }

    fn render_image_card(&mut self, ui: &mut egui::Ui) {
        let t = self.state.t();
        let colors = self.theme.colors();
        egui::Frame::none()
            .fill(colors.card_bg)
            .stroke(Stroke::new(1.0_f32, colors.card_stroke))
            .rounding(Rounding::same(6.0))
            .inner_margin(10.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(t.lbl_image).strong());

                    if let Some(path) = &self.state.image_path {
                        let name = self
                            .state
                            .image_work_name
                            .as_deref()
                            .unwrap_or(t.image_no_name);
                        ui.label(
                            RichText::new(format!("[{}]", name))
                                .strong()
                                .color(Color32::from_rgb(218, 112, 214)),
                        );
                        ui.label(
                            RichText::new(path.file_name().unwrap_or_default().to_string_lossy())
                                .strong(),
                        )
                        .on_hover_text(path.display().to_string());

                        if self.state.image_size > 0 {
                            ui.label(
                                RichText::new(format!(
                                    "({:.2} GB)",
                                    self.state.image_size as f64 / 1e9
                                ))
                                .weak(),
                            );
                        }
                    } else {
                        ui.label(RichText::new(t.image_none).weak());
                    }

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button(t.gui_btn_open_once).clicked() {
                            if let Some(path) = mass_usb_flasher::picker::pick_image_file() {
                                self.state.select_image_path(path, false);
                            } else {
                                self.state.file_picker_mode = mass_usb_flasher::app::FilePickerMode::OpenOnce;
                                self.state.refresh_file_picker_entries();
                                self.show_file_picker_modal = true;
                            }
                        }
                        if ui.button(t.gui_btn_catalog).clicked() {
                            self.show_catalog_modal = true;
                        }
                        if ui
                            .button(RichText::new(t.gui_btn_select).strong())
                            .clicked()
                        {
                            if let Some(path) = mass_usb_flasher::picker::pick_image_file() {
                                self.state.select_image_path(path, true);
                            } else {
                                self.state.file_picker_mode = mass_usb_flasher::app::FilePickerMode::AddImage;
                                self.state.refresh_file_picker_entries();
                                self.show_file_picker_modal = true;
                            }
                        }
                    });
                });

                // Строка SHA-256
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(t.lbl_sha256).weak());
                    if let Some(hash) = &self.state.image_sha256 {
                        ui.label(
                            RichText::new(hash)
                                .monospace()
                                .color(Color32::from_rgb(76, 175, 80)),
                        );
                    } else if let Some((done, total)) = self.state.image_hash_progress {
                        let pct = if total > 0 {
                            done as f32 / total as f32
                        } else {
                            0.0
                        };
                        ui.add(
                            egui::ProgressBar::new(pct)
                                .text(format!(
                                    "{}: {:.1}% ({:.0}/{:.0} MB)",
                                    t.sha_computing,
                                    pct * 100.0,
                                    done as f64 / 1_048_576.0,
                                    total as f64 / 1_048_576.0
                                ))
                                .desired_width(320.0),
                        );
                    } else if self.state.image_path.is_some() {
                        ui.label(RichText::new(t.sha_starting).italics().weak());
                    } else {
                        ui.label(RichText::new("—").weak());
                    }
                });
            });
    }

    fn render_devices_table(&mut self, ui: &mut egui::Ui) {
        let t = self.state.t();
        let colors = self.theme.colors();
        let ready_count = self.state.ready_count();
        let target = self.state.target_count;
        let total_count = self.state.devices.len();

        ui.horizontal(|ui| {
            ui.heading(RichText::new(t.gui_title_devices).size(17.0).strong());
            ui.label(
                RichText::new(format!(
                    "({} connected, {}/{} ready)",
                    total_count, ready_count, target
                ))
                .weak(),
            );
        });

        ui.add_space(6.0);

        if self.state.devices.is_empty() {
            egui::Frame::none()
                .fill(colors.empty_box_bg)
                .stroke(Stroke::new(1.0_f32, colors.card_stroke))
                .rounding(Rounding::same(6.0))
                .inner_margin(40.0)
                .show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.label(
                            RichText::new("🔌")
                                .size(36.0)
                                .color(colors.empty_icon_color),
                        );
                        ui.add_space(8.0);
                        ui.label(RichText::new(t.gui_no_devices).size(15.0).weak());
                    });
                });
            return;
        }

        let mut manual_retry_slot = None;

        // Таблица устройств
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Grid::new("devices_grid")
                    .striped(true)
                    .spacing(Vec2::new(14.0, 10.0))
                    .min_col_width(50.0)
                    .show(ui, |ui| {
                        // Заголовки таблицы
                        ui.label(RichText::new(t.th_id).strong().weak());
                        ui.label(RichText::new(t.th_path).strong().weak());
                        ui.label(RichText::new(t.th_size).strong().weak());
                        ui.label(RichText::new(t.th_product).strong().weak());
                        ui.label(RichText::new(t.th_serial).strong().weak());
                        ui.label(RichText::new(t.th_status).strong().weak());
                        ui.label(RichText::new(t.th_progress).strong().weak());
                        ui.end_row();

                        for (n, slot) in &self.state.devices {
                            let gb = slot.info.size_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
                            let (status_str, status_color) = match slot.status {
                                SlotStatus::Waiting => (t.slot_waiting, Color32::from_rgb(120, 120, 130)),
                                SlotStatus::Ready => (t.slot_ready, Color32::from_rgb(76, 175, 80)),
                                SlotStatus::Writing => (t.slot_writing, Color32::from_rgb(33, 150, 243)),
                                SlotStatus::Verifying => (t.slot_verifying, Color32::from_rgb(255, 193, 7)),
                                SlotStatus::Formatting => (t.slot_formatting, Color32::from_rgb(255, 152, 0)),
                                SlotStatus::Done => (t.slot_done, Color32::from_rgb(76, 175, 80)),
                                SlotStatus::Failed => (t.slot_failed, Color32::from_rgb(244, 67, 54)),
                                SlotStatus::Retrying => (t.slot_retrying, Color32::from_rgb(255, 179, 0)),
                            };

                            ui.label(RichText::new(format!("#{}", n)).strong());
                            ui.label(RichText::new(&slot.info.path).monospace());
                            ui.label(format!("{:.2} GB", gb));
                            ui.label(&slot.info.product);
                            ui.label(RichText::new(&slot.info.serial).weak());

                            // Статус бейдж
                            ui.label(RichText::new(status_str).strong().color(status_color));

                            // Прогресс бар
                            match slot.status {
                                SlotStatus::Writing | SlotStatus::Verifying | SlotStatus::Formatting => {
                                    ui.add(
                                        egui::ProgressBar::new(slot.progress)
                                            .text(format!("{:>5.1}%", slot.progress * 100.0))
                                            .desired_width(160.0),
                                    );
                                }
                                SlotStatus::Retrying => {
                                    ui.horizontal(|ui| {
                                        ui.add(
                                            egui::ProgressBar::new(slot.progress)
                                                .text(format!("{:>5.1}%", slot.progress * 100.0))
                                                .desired_width(120.0),
                                        );
                                        let msg = if slot.message.is_empty() { "Retrying…" } else { &slot.message };
                                        ui.label(RichText::new(msg).color(Color32::from_rgb(255, 179, 0)).size(11.0));
                                    });
                                }
                                SlotStatus::Done => {
                                    ui.label(RichText::new("100.0%  ✔").strong().color(Color32::from_rgb(76, 175, 80)));
                                }
                                SlotStatus::Failed => {
                                    ui.horizontal(|ui| {
                                        let msg = if slot.message.is_empty() { "ERROR" } else { &slot.message };
                                        ui.label(RichText::new(msg).color(Color32::from_rgb(244, 67, 54)).size(11.0));
                                        if ui.small_button("🔄 Retry").on_hover_text("Retry writing this flash drive individually").clicked() {
                                            manual_retry_slot = Some(*n);
                                        }
                                    });
                                }
                                _ => {
                                    ui.label("—");
                                }
                            }

                            ui.end_row();
                        }
                    });
            });

        if let Some(disk_num) = manual_retry_slot {
            self.state.manual_retry_device(disk_num);
        }
    }

    fn render_bottom_bar(&mut self, ui: &mut egui::Ui) {
        let t = self.state.t();

        // Строка времени и скорости во время или после записи
        if self.state.status == AppStatus::Finished {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("🎉 {}", t.gui_all_done))
                        .strong()
                        .size(15.0)
                        .color(Color32::from_rgb(186, 104, 200)),
                );
                if let Some(elapsed) = self.state.elapsed_secs() {
                    ui.label(RichText::new(format!("({}: {})", t.lbl_elapsed, format_elapsed(elapsed))).weak());
                }
            });
            ui.add_space(4.0);
        } else if let Some(elapsed) = self.state.elapsed_secs() {
            let speed = self
                .state
                .per_device_speed_mbps()
                .map(|s| format!("{:.1} {}", s, t.speed_per_device))
                .unwrap_or_else(|| "—".to_string());
            let avg_bytes = self.state.average_bytes_written();
            let total = self.state.write_total_bytes;

            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{}: {}", t.lbl_elapsed, format_elapsed(elapsed))).weak());
                ui.separator();
                ui.label(RichText::new(format!("{}: {}", t.lbl_speed, speed)).color(Color32::from_rgb(33, 150, 243)));
                ui.separator();
                ui.label(RichText::new(format!("{}: {:.2} / {:.2} GB", t.lbl_written, avg_bytes as f64 / 1e9, total as f64 / 1e9)).weak());
            });
            ui.add_space(4.0);
        }

        // КНОПКИ ДЕЙСТВИЙ
        ui.horizontal(|ui| {
            // Кнопка ЗАПУСКА ЗАПИСИ
            let can_start = self.state.status == AppStatus::Ready && self.state.image_sha256.is_some();
            let start_btn = egui::Button::new(
                RichText::new(t.gui_btn_start)
                    .size(15.0)
                    .strong()
                    .color(if can_start { Color32::WHITE } else { Color32::from_rgb(140, 140, 140) }),
            )
            .fill(if can_start { Color32::from_rgb(46, 125, 50) } else { Color32::from_rgb(45, 50, 45) })
            .min_size(Vec2::new(180.0, 36.0));

            if ui.add_enabled(can_start, start_btn).clicked() {
                self.state.start_write_flow();
            }

            // КНОПКА СБРОСА / ПЕРЕКЛЮЧЕНИЯ В IDLE (READY) ДЛЯ СЛЕДУЮЩЕЙ ПАРТИИ
            // Когда запись завершена, кнопка выделяется ярким фиолетовым/зеленым цветом
            let is_finished = self.state.status == AppStatus::Finished;
            let reset_btn = egui::Button::new(
                RichText::new(t.gui_btn_reset)
                    .size(15.0)
                    .strong()
                    .color(if is_finished { Color32::WHITE } else { Color32::from_rgb(200, 200, 220) }),
            )
            .fill(if is_finished { Color32::from_rgb(142, 36, 170) } else { Color32::from_rgb(50, 52, 62) })
            .min_size(Vec2::new(220.0, 36.0));

            if ui.add(reset_btn).on_hover_text("Reset device statuses and switch back to Ready/Idle for the next batch of flash drives").clicked() {
                self.state.reset_to_idle();
                self.taskbar.clear();
            }

            ui.add_space(10.0);

            if ui.button(if self.show_log_panel { "📜 Hide Log" } else { "📜 Log" }).clicked() {
                self.show_log_panel = !self.show_log_panel;
            }

            if ui.button("❓ Help").clicked() {
                self.show_help_modal = true;
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button(RichText::new(t.fk_quit).color(Color32::from_rgb(255, 100, 100))).clicked() {
                    std::process::exit(0);
                }

                // Вотермарка со ссылкой
                ui.hyperlink_to(
                    RichText::new(WATERMARK).weak().size(12.0),
                    WATERMARK_URL,
                );
            });
        });
    }

    fn render_log_viewer(&mut self, ui: &mut egui::Ui) {
        let t = self.state.t();
        // Панель заголовка и управления логом
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("📜 {}", t.gui_title_log)).strong().size(13.0));
            ui.label(RichText::new(format!("({} entries)", self.state.log.len())).weak().size(11.0));

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.small_button("✕ Close").clicked() {
                    self.show_log_panel = false;
                }
                if ui.small_button(t.gui_log_clear).clicked() {
                    self.state.log.clear();
                }
                ui.checkbox(&mut self.log_autoscroll, t.gui_log_autoscroll);
            });
        });

        ui.add_space(2.0);

        egui::ScrollArea::vertical()
            .stick_to_bottom(self.log_autoscroll)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for line in &self.state.log {
                    let color = if line.contains("[!]") || line.contains("error") || line.contains("Failed") {
                        Color32::from_rgb(244, 67, 54)
                    } else if line.contains("===") || line.contains("Starting") || line.contains("Preparing") {
                        Color32::from_rgb(100, 200, 255)
                    } else if line.contains("[+]") || line.contains("complete") || line.contains("succeeded") || line.contains("✔") {
                        Color32::from_rgb(76, 175, 80)
                    } else if line.contains("Retry") || line.contains("Reconnecting") || line.contains("Waiting") {
                        Color32::from_rgb(255, 179, 0)
                    } else {
                        Color32::from_rgb(180, 185, 195)
                    };
                    ui.label(RichText::new(line).monospace().size(11.5).color(color));
                }
            });
    }

    fn render_catalog_window(&mut self, ctx: &egui::Context) {
        let t = self.state.t();
        let mut open = true;
        let mut select_index = None;
        let mut delete_index = None;

        egui::Window::new(t.title_catalog_fmt.replace("{}", &self.state.catalog.images.len().to_string()))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size([720.0, 420.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.button(t.cat_hint_a).clicked() {
                        if let Some(path) = mass_usb_flasher::picker::pick_image_file() {
                            self.state.select_image_path(path, true);
                        } else {
                            self.state.file_picker_mode = mass_usb_flasher::app::FilePickerMode::AddImage;
                            self.state.refresh_file_picker_entries();
                            self.show_file_picker_modal = true;
                        }
                    }
                    if ui.button(t.cat_hint_f).clicked() {
                        if let Some(folder) = mass_usb_flasher::picker::pick_image_folder() {
                            self.state.apply_import_folder(folder);
                        } else {
                            self.state.file_picker_mode = mass_usb_flasher::app::FilePickerMode::ImportFolder;
                            self.state.refresh_file_picker_entries();
                            self.show_file_picker_modal = true;
                        }
                    }
                });
                ui.separator();

                egui::ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
                    egui::Grid::new("catalog_grid")
                        .striped(true)
                        .spacing(Vec2::new(12.0, 8.0))
                        .show(ui, |ui| {
                            ui.label(RichText::new("Work Name").strong());
                            ui.label(RichText::new("Size").strong());
                            ui.label(RichText::new("SHA256").strong());
                            ui.label(RichText::new("Actions").strong());
                            ui.end_row();

                            for (i, entry) in self.state.catalog.images.iter().enumerate() {
                                ui.label(RichText::new(&entry.work_name).strong());
                                ui.label(format!("{:.2} GB", entry.size_bytes as f64 / 1e9));
                                let short_hash = if entry.sha256.len() >= 12 {
                                    format!("{}…", &entry.sha256[..12])
                                } else {
                                    entry.sha256.clone()
                                };
                                ui.label(RichText::new(short_hash).monospace())
                                    .on_hover_text(&entry.sha256);

                                ui.horizontal(|ui| {
                                    if ui.button("✔ Select").clicked() {
                                        select_index = Some(i);
                                    }
                                    if ui.button("🗑").clicked() {
                                        delete_index = Some(i);
                                    }
                                });
                                ui.end_row();
                            }
                        });
                });
            });

        if let Some(idx) = select_index {
            let _ = self.state.select_image_from_catalog(idx);
            self.show_catalog_modal = false;
        }

        if let Some(idx) = delete_index {
            self.state.image_select_index = idx;
            self.state.delete_selected_from_catalog();
        }

        if !open {
            self.show_catalog_modal = false;
        }
    }

    fn render_help_window(&mut self, ctx: &egui::Context) {
        let t = self.state.t();
        let mut open = true;

        egui::Window::new(t.title_help)
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_size([540.0, 420.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.heading("Mass USB Flasher");
                ui.label("Mass USB Flash Drive Imaging Utility");
                ui.separator();

                ui.label(RichText::new("Workflow:").strong());
                ui.label("1. Connect target USB drives (they will appear in the table).");
                ui.label("2. Select an image file (Open Image or Catalog).");
                ui.label("3. Wait for SHA-256 computation to finish.");
                ui.label("4. Adjust target count if needed (+/- buttons).");
                ui.label("5. Click [Start Flashing].");
                ui.label("6. Once write is complete, click [Ready for Next Batch] to flash the next batch!");

                ui.add_space(8.0);
                ui.label(RichText::new("Key Features:").strong());
                ui.label("• Zero-overhead broadcast writer using lock-free bus_writer");
                ui.label("• Windows raw disk offline bypass (IOCTL_DISK_SET_DISK_ATTRIBUTES)");
                ui.label("• Taskbar and window progress integration");
                ui.label("• Multi-language interface (EN / RU)");
                ui.separator();

                ui.vertical_centered(|ui| {
                    ui.hyperlink_to(WATERMARK, WATERMARK_URL);
                });
            });

        if !open {
            self.show_help_modal = false;
        }
    }

    fn render_file_picker_window(&mut self, ctx: &egui::Context) {
        let t = self.state.t();
        let mut open = true;

        egui::Window::new(format!("📁 {}", t.gui_file_picker_title))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size([720.0, 480.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.button(t.gui_up_dir).clicked() {
                        if let Some(parent) = self.state.file_picker_dir.parent().map(|p| p.to_path_buf()) {
                            self.state.file_picker_dir = parent;
                            self.state.refresh_file_picker_entries();
                        }
                    }

                    ui.label(RichText::new(self.state.file_picker_dir.display().to_string()).strong().monospace());

                    if self.state.file_picker_mode == mass_usb_flasher::app::FilePickerMode::ImportFolder {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.button(RichText::new(t.gui_btn_select_dir).strong()).clicked() {
                                let dir = self.state.file_picker_dir.clone();
                                self.state.apply_import_folder(dir);
                                self.show_file_picker_modal = false;
                            }
                        });
                    }
                });

                ui.separator();

                egui::ScrollArea::vertical().max_height(340.0).show(ui, |ui| {
                    let mut chosen_entry = None;
                    for item in &self.state.file_picker_entries {
                        let prefix = if item.is_dir { "📁 " } else { "📄 " };
                        let label_text = if item.is_dir {
                            format!("{}{}", prefix, item.name)
                        } else {
                            format!("{}{} ({:.2} GB)", prefix, item.name, item.size as f64 / 1e9)
                        };

                        if ui.selectable_label(false, label_text).clicked() {
                            chosen_entry = Some(item.clone());
                        }
                    }

                    if let Some(item) = chosen_entry {
                        if item.is_dir {
                            self.state.file_picker_dir = item.path;
                            self.state.refresh_file_picker_entries();
                        } else {
                            match self.state.file_picker_mode {
                                mass_usb_flasher::app::FilePickerMode::AddImage => {
                                    self.state.select_image_path(item.path, true);
                                    self.show_file_picker_modal = false;
                                }
                                mass_usb_flasher::app::FilePickerMode::OpenOnce => {
                                    self.state.select_image_path(item.path, false);
                                    self.show_file_picker_modal = false;
                                }
                                mass_usb_flasher::app::FilePickerMode::ImportFolder => {}
                            }
                        }
                    }
                });
            });

        if !open {
            self.show_file_picker_modal = false;
        }
    }
}
