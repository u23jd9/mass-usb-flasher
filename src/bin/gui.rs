#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![allow(dead_code)]

use std::time::{Duration, Instant};

use eframe::egui::{self, Align, Color32, Layout, RichText, Rounding, Stroke, Vec2};

use mass_usb_flasher::app::{AppState, AppStatus, SlotStatus};
use mass_usb_flasher::catalog::Catalog;
use mass_usb_flasher::common::{
    compute_window_title, ensure_automount_enabled, format_elapsed, is_admin, parse_args,
    parse_target_count_arg,
};
use mass_usb_flasher::device::watcher::{DeviceFilter, WatcherConfig, WatcherHandle};
use mass_usb_flasher::i18n::{VERSION, WATERMARK, WATERMARK_URL};
use mass_usb_flasher::taskbar::Taskbar;

fn main() -> eframe::Result<()> {
    if !is_admin() {
        rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Error)
            .set_title("Administrator Rights Required")
            .set_description(
                "Mass USB Flasher requires administrator rights to access physical disk drives.\n\
                 Please right-click and 'Run as administrator'.",
            )
            .show();
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
    show_catalog_modal: bool,
    show_help_modal: bool,
    show_log_panel: bool,
    last_taskbar_update: Instant,
    last_taskbar_pct: i32,
}

impl MufGuiApp {
    fn new(cc: &eframe::CreationContext) -> Self {
        let mut visuals = egui::Visuals::dark();
        visuals.window_rounding = Rounding::same(8.0);
        visuals.panel_fill = Color32::from_rgb(26, 27, 32);
        visuals.override_text_color = Some(Color32::from_rgb(230, 230, 235));
        cc.egui_ctx.set_visuals(visuals);

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
            show_catalog_modal: false,
            show_help_modal: false,
            show_log_panel: false,
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
        {
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        // --- ВЕРХНЯЯ ПАНЕЛЬ (Header & Image Controls) ---
        egui::TopBottomPanel::top("top_panel")
            .frame(
                egui::Frame::none()
                    .fill(Color32::from_rgb(32, 33, 40))
                    .inner_margin(12.0),
            )
            .show(ctx, |ui| {
                self.render_top_bar(ui);
                ui.add_space(8.0);
                self.render_image_card(ui);
            });

        // --- НИЖНЯЯ ПАНЕЛЬ (Actions & Status) ---
        egui::TopBottomPanel::bottom("bottom_panel")
            .frame(
                egui::Frame::none()
                    .fill(Color32::from_rgb(22, 23, 28))
                    .inner_margin(12.0),
            )
            .show(ctx, |ui| {
                self.render_bottom_bar(ui);
            });

        // --- ЦЕНТРАЛЬНАЯ ПАНЕЛЬ (Таблица устройств) ---
        egui::CentralPanel::default().show(ctx, |ui| {
            self.render_devices_table(ui);

            if self.show_log_panel {
                ui.separator();
                self.render_log_viewer(ui);
            }
        });

        // --- МОДАЛЬНЫЕ ОКНА ---
        if self.show_catalog_modal {
            self.render_catalog_window(ctx);
        }

        if self.show_help_modal {
            self.render_help_window(ctx);
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

            ui.painter().rect_filled(
                ui.available_rect_before_wrap(),
                Rounding::same(4.0),
                bg_color,
            );

            ui.add(
                egui::Button::new(
                    RichText::new(format!("  {}  ", status_text))
                        .strong()
                        .color(fg_color),
                )
                .fill(bg_color)
                .stroke(Stroke::NONE),
            );

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

            // Переключатель языка
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .button(
                        RichText::new(format!("🌐 {}", self.state.lang.code()))
                            .color(Color32::from_rgb(150, 200, 255)),
                    )
                    .clicked()
                {
                    self.state.lang = self.state.lang.toggle();
                }
            });
        });
    }

    fn render_image_card(&mut self, ui: &mut egui::Ui) {
        let t = self.state.t();
        egui::Frame::none()
            .fill(Color32::from_rgb(38, 40, 48))
            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(55, 58, 68)))
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
                                .strong()
                                .color(Color32::WHITE),
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
                            self.state.open_ephemeral_file_dialog();
                        }
                        if ui.button(t.gui_btn_catalog).clicked() {
                            self.show_catalog_modal = true;
                        }
                        if ui
                            .button(RichText::new(t.gui_btn_select).strong())
                            .clicked()
                        {
                            self.state.open_add_file_dialog();
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
                .fill(Color32::from_rgb(30, 31, 38))
                .rounding(Rounding::same(6.0))
                .inner_margin(40.0)
                .show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.label(
                            RichText::new("🔌")
                                .size(36.0)
                                .color(Color32::from_rgb(120, 120, 140)),
                        );
                        ui.add_space(8.0);
                        ui.label(RichText::new(t.gui_no_devices).size(15.0).weak());
                    });
                });
            return;
        }

        // Таблица устройств
        egui::ScrollArea::vertical()
            .max_height(340.0)
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
                                SlotStatus::Done => {
                                    ui.label(RichText::new("100.0%  ✔").strong().color(Color32::from_rgb(76, 175, 80)));
                                }
                                SlotStatus::Failed => {
                                    let msg = if slot.message.is_empty() { "ERROR" } else { &slot.message };
                                    ui.label(RichText::new(msg).color(Color32::from_rgb(244, 67, 54)));
                                }
                                _ => {
                                    ui.label("—");
                                }
                            }

                            ui.end_row();
                        }
                    });
            });
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
        ui.label(RichText::new(self.state.t().gui_title_log).strong().weak());
        egui::ScrollArea::vertical()
            .max_height(120.0)
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for line in self.state.log.iter().rev().take(30).collect::<Vec<_>>().into_iter().rev() {
                    let color = if line.contains("[!]") || line.contains("error") {
                        Color32::from_rgb(244, 67, 54)
                    } else if line.contains("===") {
                        Color32::from_rgb(100, 200, 255)
                    } else {
                        Color32::from_rgb(180, 180, 190)
                    };
                    ui.label(RichText::new(line).monospace().size(12.0).color(color));
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
                        self.state.open_add_file_dialog();
                    }
                    if ui.button(t.cat_hint_f).clicked() {
                        self.state.open_import_folder_dialog();
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
}
