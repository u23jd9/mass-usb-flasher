#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![allow(dead_code)]

use std::time::{Duration, Instant};

use eframe::egui::{self, Align, Align2, Color32, Layout, RichText, Rounding, Stroke, Vec2};

use mass_usb_flasher::app::{AppState, AppStatus, SlotStatus};
use mass_usb_flasher::benchmark::{BenchmarkConcurrency, BenchmarkMode, BenchmarkPhase, BenchmarkTestSize};
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

fn load_app_icon() -> Option<egui::IconData> {
    let png_bytes = include_bytes!("../../icon.png");
    if let Ok(img) = image::load_from_memory(png_bytes) {
        let rgba = img.to_rgba8();
        let (width, height) = rgba.dimensions();
        Some(egui::IconData {
            rgba: rgba.into_raw(),
            width,
            height,
        })
    } else {
        None
    }
}

fn main() -> eframe::Result<()> {
    if !ensure_elevated(true) {
        std::process::exit(1);
    }

    let _ = ensure_automount_enabled();

    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Mass USB Flasher")
        .with_inner_size([1080.0, 720.0])
        .with_min_inner_size([900.0, 560.0]);

    if let Some(icon) = load_app_icon() {
        viewport = viewport.with_icon(icon);
    }

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "Mass USB Flasher",
        options,
        Box::new(|cc| Ok(Box::new(MufGuiApp::new(cc)))),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiScaleMode {
    Auto,
    Scale100,
    Scale125,
    Scale150,
    Scale175,
    Scale200,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppTab {
    Flashing,
    Benchmark,
}

struct MufGuiApp {
    state: AppState,
    watcher: WatcherHandle,
    taskbar: Taskbar,
    theme: GuiTheme,
    current_tab: AppTab,
    show_catalog_modal: bool,
    show_help_modal: bool,
    show_file_picker_modal: bool,
    show_log_panel: bool,
    log_autoscroll: bool,
    ui_scale_mode: UiScaleMode,
    current_zoom: f32,
    show_images_sidebar: bool,
    sidebar_search: String,
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
            current_tab: AppTab::Flashing,
            show_catalog_modal: false,
            show_help_modal: false,
            show_file_picker_modal: false,
            show_log_panel: false,
            log_autoscroll: true,
            ui_scale_mode: UiScaleMode::Auto,
            current_zoom: 1.0,
            show_images_sidebar: true,
            sidebar_search: String::new(),
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
        // Адаптивное масштабирование интерфейса
        let (viewport_w, viewport_h) = {
            let rect = ctx.screen_rect();
            (rect.width(), rect.height())
        };

        let desired_zoom = match self.ui_scale_mode {
            UiScaleMode::Auto => {
                let phys_w = viewport_w * self.current_zoom;
                let phys_h = viewport_h * self.current_zoom;
                if phys_w >= 2400.0 || phys_h >= 1350.0 {
                    1.6
                } else if phys_w >= 1800.0 || phys_h >= 1000.0 {
                    1.3
                } else if phys_w >= 1400.0 || phys_h >= 850.0 {
                    1.15
                } else {
                    1.0
                }
            }
            UiScaleMode::Scale100 => 1.0,
            UiScaleMode::Scale125 => 1.25,
            UiScaleMode::Scale150 => 1.5,
            UiScaleMode::Scale175 => 1.75,
            UiScaleMode::Scale200 => 2.0,
        };

        if (desired_zoom - self.current_zoom).abs() > 0.02 {
            self.current_zoom = desired_zoom;
            ctx.set_zoom_factor(desired_zoom);
        }

        // Фоновая обработка очередей
        self.state.drain_device_events(&self.watcher);
        self.state.drain_hash_job();
        self.state.drain_import_job();
        self.state.drain_prepare();
        self.state.drain_write();
        self.state.drain_single_retries();
        self.state.drain_benchmark();
        self.state.auto_transition_finished();
        self.update_taskbar();

        // Задаём заголовок окна
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(compute_window_title(
            &self.state,
        )));

        // Во время записи, бенчмарка или хеширования перерисовываем чаще
        if matches!(self.state.status, AppStatus::Running)
            || self.state.image_hash_progress.is_some()
            || self.state.import_job.is_some()
            || self.state.has_active_retries()
            || self.state.benchmark_active
        {
            ctx.request_repaint_after(Duration::from_millis(50));
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

        // --- ВЕРХНЯЯ ПАНЕЛЬ (Header & Image/Benchmark Controls) ---
        egui::TopBottomPanel::top("top_panel")
            .frame(
                egui::Frame::none()
                    .fill(colors.top_panel_bg)
                    .inner_margin(12.0),
            )
            .show(ctx, |ui| {
                self.render_top_bar(ui);
                ui.add_space(8.0);
                match self.current_tab {
                    AppTab::Flashing => self.render_image_card(ui),
                    AppTab::Benchmark => self.render_benchmark_top_card(ui),
                }
            });

        // --- НИЖНЯЯ ПАНЕЛЬ ДЕЙСТВИЙ (Start, Reset, Help, Status) ---
        egui::TopBottomPanel::bottom("bottom_panel")
            .frame(
                egui::Frame::none()
                    .fill(colors.bottom_panel_bg)
                    .inner_margin(12.0),
            )
            .show(ctx, |ui| {
                match self.current_tab {
                    AppTab::Flashing => self.render_bottom_bar(ui),
                    AppTab::Benchmark => self.render_benchmark_bottom_bar(ui),
                }
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

        // --- САЙД-ПАНЕЛЬ БИБЛИОТЕКИ ОБРАЗОВ (Коллапсируемая панель слева) ---
        if self.show_images_sidebar && self.current_tab == AppTab::Flashing {
            egui::SidePanel::left("images_sidebar_panel")
                .resizable(true)
                .default_width(280.0)
                .min_width(220.0)
                .max_width(450.0)
                .frame(
                    egui::Frame::none()
                        .fill(colors.card_bg)
                        .stroke(Stroke::new(1.0_f32, colors.card_stroke))
                        .inner_margin(10.0),
                )
                .show(ctx, |ui| {
                    self.render_images_sidebar(ui);
                });
        }

        // --- ЦЕНТРАЛЬНАЯ ПАНЕЛЬ (Таблица устройств / Бенчмарк) ---
        egui::CentralPanel::default().show(ctx, |ui| {
            match self.current_tab {
                AppTab::Flashing => self.render_devices_table(ui),
                AppTab::Benchmark => self.render_benchmark_table(ui),
            }
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
            // Левая часть: Название приложения, версия и вкладки
            ui.heading(
                RichText::new("⚡ Mass USB Flasher")
                    .strong()
                    .color(Color32::from_rgb(100, 200, 255)),
            );
            ui.label(RichText::new(format!("v{}", VERSION)).weak());

            ui.add_space(8.0);

            // --- TAB SWITCHER ---
            let is_flashing = self.current_tab == AppTab::Flashing;
            if ui
                .selectable_label(
                    is_flashing,
                    RichText::new(format!("🔥 {}", t.tab_flashing)).strong().size(13.5),
                )
                .clicked()
            {
                self.current_tab = AppTab::Flashing;
            }

            let is_bench = self.current_tab == AppTab::Benchmark;
            let bench_label = if self.state.benchmark_active {
                format!("⚡ {} ●", t.tab_benchmark)
            } else {
                format!("⚡ {}", t.tab_benchmark)
            };
            if ui
                .selectable_label(
                    is_bench,
                    RichText::new(bench_label)
                        .strong()
                        .size(13.5)
                        .color(if self.state.benchmark_active {
                            Color32::from_rgb(255, 193, 7)
                        } else if is_bench {
                            Color32::from_rgb(100, 200, 255)
                        } else {
                            Color32::from_rgb(200, 205, 215)
                        }),
                )
                .clicked()
            {
                self.current_tab = AppTab::Benchmark;
            }

            ui.add_space(8.0);

            // Правая часть: элементы управления языком, темой, масштабом
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

                egui::ComboBox::from_id_salt("ui_scale_selector")
                    .selected_text(match self.ui_scale_mode {
                        UiScaleMode::Auto => format!("🔍 Auto ({:.0}%)", self.current_zoom * 100.0),
                        UiScaleMode::Scale100 => "🔍 100%".into(),
                        UiScaleMode::Scale125 => "🔍 125%".into(),
                        UiScaleMode::Scale150 => "🔍 150%".into(),
                        UiScaleMode::Scale175 => "🔍 175%".into(),
                        UiScaleMode::Scale200 => "🔍 200%".into(),
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.ui_scale_mode, UiScaleMode::Auto, "Auto (Adaptive)");
                        ui.selectable_value(&mut self.ui_scale_mode, UiScaleMode::Scale100, "100%");
                        ui.selectable_value(&mut self.ui_scale_mode, UiScaleMode::Scale125, "125%");
                        ui.selectable_value(&mut self.ui_scale_mode, UiScaleMode::Scale150, "150%");
                        ui.selectable_value(&mut self.ui_scale_mode, UiScaleMode::Scale175, "175%");
                        ui.selectable_value(&mut self.ui_scale_mode, UiScaleMode::Scale200, "200%");
                    });

                ui.separator();

                // Центр/оставшееся пространство справа налево: целевое кол-во и статус-бейдж
                if self.current_tab == AppTab::Flashing {
                    let auto_color = if self.state.autotarget {
                        Color32::from_rgb(76, 175, 80)
                    } else {
                        Color32::from_rgb(130, 135, 145)
                    };
                    if ui
                        .small_button(RichText::new(format!("[{}]", t.gui_autotarget_btn)).color(auto_color).strong())
                        .on_hover_text("Auto-target: matches total connected USB drives automatically")
                        .clicked()
                    {
                        self.state.enable_autotarget();
                    }

                    if ui.button("➕").clicked() {
                        self.state.increase_target();
                    }
                    ui.label(
                        RichText::new(format!("{}", self.state.target_count))
                            .strong()
                            .size(15.0)
                            .color(Color32::from_rgb(255, 215, 0)),
                    );
                    if ui.button("➖").clicked() {
                        self.state.decrease_target();
                    }
                    ui.label(RichText::new(t.lbl_target).strong());

                    ui.add_space(6.0);

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
                        .inner_margin(egui::Margin::symmetric(8.0, 3.0))
                        .show(ui, |ui| {
                            ui.label(RichText::new(status_text).strong().size(12.0).color(fg_color));
                        });

                    // Кнопка переключения сайдбара библиотеки образов
                    if ui
                        .selectable_label(
                            self.show_images_sidebar,
                            RichText::new(format!("{} ({})", t.gui_sidebar_toggle, self.state.catalog.images.len()))
                                .strong(),
                        )
                        .clicked()
                    {
                        self.show_images_sidebar = !self.show_images_sidebar;
                    }
                } else {
                    // Статус-бейдж для вкладки бенчмарка
                    let (status_text, bg_color, fg_color) = if self.state.benchmark_active {
                        (t.bench_summary_running, Color32::from_rgb(180, 110, 0), Color32::WHITE)
                    } else if self.state.devices.is_empty() {
                        (t.bench_no_devices, Color32::from_rgb(60, 60, 70), Color32::WHITE)
                    } else {
                        (t.bench_status_idle, Color32::from_rgb(34, 139, 34), Color32::WHITE)
                    };

                    egui::Frame::none()
                        .fill(bg_color)
                        .rounding(Rounding::same(4.0))
                        .inner_margin(egui::Margin::symmetric(8.0, 3.0))
                        .show(ui, |ui| {
                            ui.label(RichText::new(status_text).strong().size(12.0).color(fg_color));
                        });
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
                        let file_name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
                        ui.add(
                            egui::Label::new(RichText::new(file_name).strong())
                                .truncate()
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
                            }
                        }
                    });
                });

                // Строка SHA-256
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(t.lbl_sha256).weak());
                    if let Some(hash) = &self.state.image_sha256 {
                        ui.add(
                            egui::Label::new(
                                RichText::new(hash)
                                    .monospace()
                                    .color(Color32::from_rgb(76, 175, 80)),
                            )
                            .truncate()
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

    fn render_benchmark_top_card(&mut self, ui: &mut egui::Ui) {
        let t = self.state.t();
        let colors = self.theme.colors();
        egui::Frame::none()
            .fill(colors.card_bg)
            .stroke(Stroke::new(1.0_f32, colors.card_stroke))
            .rounding(Rounding::same(6.0))
            .inner_margin(10.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new(t.bench_title)
                                .strong()
                                .size(14.0)
                                .color(Color32::from_rgb(100, 200, 255)),
                        );
                        ui.label(
                            RichText::new(t.bench_subtitle)
                                .size(11.0)
                                .weak(),
                        );
                    });

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        // Чекбокс затирания таблицы разделов
                        ui.checkbox(
                            &mut self.state.benchmark_config.wipe_partitions,
                            RichText::new(t.bench_wipe_partitions).strong().size(12.0),
                        ).on_hover_text("Crucial: writes zeroes to sector 0 (MBR/GPT) and backup GPT at the end of the disk so OS releases volume locks");

                        ui.add_space(10.0);

                        let lang = self.state.lang;

                        // Выбор режима очередности (Последовательно / Параллельно)
                        let conc_label = match lang {
                            mass_usb_flasher::i18n::Lang::Ru => self.state.benchmark_config.concurrency.label_ru(),
                            mass_usb_flasher::i18n::Lang::En => self.state.benchmark_config.concurrency.label_en(),
                        };

                        egui::ComboBox::from_id_salt("bench_concurrency_combo")
                            .selected_text(RichText::new(conc_label).strong().color(Color32::from_rgb(76, 175, 80)))
                            .show_ui(ui, |ui| {
                                for &c in &[BenchmarkConcurrency::Sequential, BenchmarkConcurrency::Parallel] {
                                    let l = match lang {
                                        mass_usb_flasher::i18n::Lang::Ru => c.label_ru(),
                                        mass_usb_flasher::i18n::Lang::En => c.label_en(),
                                    };
                                    ui.selectable_value(&mut self.state.benchmark_config.concurrency, c, l);
                                }
                            })
                            .response
                            .on_hover_text("Sequential: tests one drive at a time. Strongly recommended when using USB hubs to prevent bus power drop and xHCI resets.");

                        ui.label(RichText::new(t.bench_lbl_concurrency).strong());

                        ui.add_space(10.0);

                        // Выбор режима (Обычный / Детальный с термодиагностикой перегрева)
                        let mode_label = match lang {
                            mass_usb_flasher::i18n::Lang::Ru => self.state.benchmark_config.mode.label_ru(),
                            mass_usb_flasher::i18n::Lang::En => self.state.benchmark_config.mode.label_en(),
                        };

                        egui::ComboBox::from_id_salt("bench_mode_combo")
                            .selected_text(RichText::new(mode_label).strong().color(Color32::from_rgb(100, 200, 255)))
                            .show_ui(ui, |ui| {
                                for &m in &[BenchmarkMode::Detailed, BenchmarkMode::Standard] {
                                    let l = match lang {
                                        mass_usb_flasher::i18n::Lang::Ru => m.label_ru(),
                                        mass_usb_flasher::i18n::Lang::En => m.label_en(),
                                    };
                                    ui.selectable_value(&mut self.state.benchmark_config.mode, m, l);
                                }
                            })
                            .response
                            .on_hover_text("Detailed mode monitors controller thermal disconnections, gives 3s cool-down pauses and auto-recovers to diagnose overheating vs hardware failure");

                        ui.label(RichText::new(t.bench_lbl_mode).strong());

                        ui.add_space(10.0);

                        // Выбор объёма теста
                        let current_size_label = match lang {
                            mass_usb_flasher::i18n::Lang::Ru => self.state.benchmark_config.test_size.label_ru(),
                            mass_usb_flasher::i18n::Lang::En => self.state.benchmark_config.test_size.label_en(),
                        };

                        egui::ComboBox::from_id_salt("bench_test_size_combo")
                            .selected_text(current_size_label)
                            .show_ui(ui, |ui| {
                                for &size in &[
                                    BenchmarkTestSize::Size256MB,
                                    BenchmarkTestSize::Size512MB,
                                    BenchmarkTestSize::Size1GB,
                                    BenchmarkTestSize::Size2GB,
                                    BenchmarkTestSize::Size4GB,
                                    BenchmarkTestSize::FullDisk,
                                ] {
                                    let label = match lang {
                                        mass_usb_flasher::i18n::Lang::Ru => size.label_ru(),
                                        mass_usb_flasher::i18n::Lang::En => size.label_en(),
                                    };
                                    ui.selectable_value(
                                        &mut self.state.benchmark_config.test_size,
                                        size,
                                        label,
                                    );
                                }
                            });
                        ui.label(RichText::new(t.bench_test_size).strong());
                    });
                });
            });
    }

    fn render_images_sidebar(&mut self, ui: &mut egui::Ui) {
        let t = self.state.t();
        let colors = self.theme.colors();

        ui.horizontal(|ui| {
            ui.heading(RichText::new(t.gui_sidebar_title).size(15.0).strong());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.small_button("✕").on_hover_text("Hide sidebar").clicked() {
                    self.show_images_sidebar = false;
                }
            });
        });

        ui.add_space(4.0);

        ui.horizontal(|ui| {
            if ui.button("➕ Add...").clicked() {
                if let Some(path) = mass_usb_flasher::picker::pick_image_file() {
                    self.state.select_image_path(path, true);
                }
            }
            if ui.button("📂 Folder...").clicked() {
                if let Some(folder) = mass_usb_flasher::picker::pick_image_folder() {
                    self.state.apply_import_folder(folder);
                }
            }
        });

        ui.add_space(6.0);
        ui.add(
            egui::TextEdit::singleline(&mut self.sidebar_search)
                .hint_text("🔍 Search images…")
                .desired_width(ui.available_width()),
        );

        ui.add_space(6.0);
        ui.separator();

        let search = self.sidebar_search.to_lowercase();
        let current_path = self.state.image_path.clone();

        let mut to_select_index = None;
        let mut to_delete_index = None;

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if self.state.catalog.images.is_empty() {
                    ui.add_space(20.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new("📁").size(28.0).weak());
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new("No images in library.\nClick 'Add' or drag-and-drop file here.")
                                .weak()
                                .size(12.0),
                        );
                    });
                    return;
                }

                for (idx, entry) in self.state.catalog.images.iter().enumerate() {
                    if !search.is_empty()
                        && !entry.work_name.to_lowercase().contains(&search)
                        && !entry.path.to_string_lossy().to_lowercase().contains(&search)
                    {
                        continue;
                    }

                    let is_selected = current_path.as_ref() == Some(&entry.path);
                    let stroke_color = if is_selected {
                        Color32::from_rgb(33, 150, 243)
                    } else {
                        colors.card_stroke
                    };
                    let bg_color = if is_selected {
                        Color32::from_rgb(28, 45, 65)
                    } else {
                        colors.empty_box_bg
                    };

                    egui::Frame::none()
                        .fill(bg_color)
                        .stroke(Stroke::new(if is_selected { 1.5_f32 } else { 1.0_f32 }, stroke_color))
                        .rounding(Rounding::same(6.0))
                        .inner_margin(8.0)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                let name_label = ui.add(
                                    egui::Label::new(
                                        RichText::new(&entry.work_name)
                                            .strong()
                                            .size(13.0)
                                            .color(if is_selected {
                                                Color32::from_rgb(100, 200, 255)
                                            } else {
                                                colors.text_primary
                                            }),
                                    )
                                    .sense(egui::Sense::click()),
                                );
                                if name_label.clicked() {
                                    to_select_index = Some(idx);
                                }

                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if ui.small_button("🗑").on_hover_text("Delete from catalog").clicked() {
                                        to_delete_index = Some(idx);
                                    }
                                    if is_selected {
                                        ui.label(RichText::new("✔").color(Color32::from_rgb(76, 175, 80)).strong());
                                    }
                                });
                            });

                            let gb = entry.size_bytes as f64 / 1e9;
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(format!("{:.2} GB", gb)).weak().size(11.0));
                                if entry.sha256.len() >= 8 {
                                    ui.label(RichText::new(format!("SHA: {}…", &entry.sha256[..8])).weak().size(11.0));
                                }
                            });

                            let btn_text = if is_selected { "✔ Active" } else { "Select" };
                            if ui.button(btn_text).clicked() {
                                to_select_index = Some(idx);
                            }
                        });
                    ui.add_space(4.0);
                }
            });

        if let Some(idx) = to_select_index {
            let _ = self.state.select_image_from_catalog(idx);
        }
        if let Some(idx) = to_delete_index {
            if idx < self.state.catalog.images.len() {
                self.state.catalog.images.remove(idx);
                let _ = self.state.catalog.save(&self.state.catalog_path);
            }
        }
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
        let mut flash_single_slot = None;
        let mut stop_beacon_slot = None;

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
                        ui.label(RichText::new("Actions").strong().weak());
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

                            let id_lbl = ui.add(
                                egui::Label::new(RichText::new(format!("#{}", n)).strong())
                                    .sense(egui::Sense::click()),
                            );
                            id_lbl.context_menu(|ui| {
                                if ui.button(t.gui_menu_flash_single).clicked() {
                                    flash_single_slot = Some(*n);
                                    ui.close_menu();
                                }
                            });

                            let path_lbl = ui.add(
                                egui::Label::new(RichText::new(&slot.info.path).monospace())
                                    .sense(egui::Sense::click()),
                            );
                            path_lbl.context_menu(|ui| {
                                if ui.button(t.gui_menu_flash_single).clicked() {
                                    flash_single_slot = Some(*n);
                                    ui.close_menu();
                                }
                            });

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
                                            .desired_width(140.0),
                                    );
                                }
                                SlotStatus::Retrying => {
                                    ui.horizontal(|ui| {
                                        ui.add(
                                            egui::ProgressBar::new(slot.progress)
                                                .text(format!("{:>5.1}%", slot.progress * 100.0))
                                                .desired_width(110.0),
                                        );
                                        let msg = if slot.message.is_empty() { "Retrying…" } else { &slot.message };
                                        ui.label(RichText::new(msg).color(Color32::from_rgb(255, 179, 0)).size(11.0));
                                    });
                                }
                                SlotStatus::Done => {
                                    ui.label(RichText::new("100.0%  ✔").strong().color(Color32::from_rgb(76, 175, 80)));
                                }
                                SlotStatus::Failed => {
                                    let msg = if slot.message.is_empty() { "ERROR" } else { &slot.message };
                                    ui.label(RichText::new(msg).color(Color32::from_rgb(244, 67, 54)).size(11.0));
                                }
                                _ => {
                                    ui.label("—");
                                }
                            }

                            // Колонка действий
                            ui.horizontal(|ui| {
                                match slot.status {
                                    SlotStatus::Failed => {
                                        if slot.message.contains("Beacon") || slot.message.contains("BEACON") {
                                            if ui.small_button(RichText::new(t.gui_btn_stop_beacon).color(Color32::from_rgb(255, 100, 100))).on_hover_text("Stop beacon blinking on card reader").clicked() {
                                                stop_beacon_slot = Some(*n);
                                            }
                                        } else if ui.small_button(t.gui_btn_retry).on_hover_text("Retry writing this flash drive individually").clicked() {
                                            manual_retry_slot = Some(*n);
                                        }
                                    }
                                    SlotStatus::Retrying => {
                                        if ui.small_button("⏹ Cancel").clicked() {
                                            stop_beacon_slot = Some(*n);
                                        }
                                    }
                                    SlotStatus::Ready | SlotStatus::Waiting => {
                                        if self.state.status != AppStatus::Running {
                                            if ui.small_button(RichText::new(t.gui_btn_flash_single).color(Color32::from_rgb(100, 200, 255))).on_hover_text("Flash only this drive").clicked() {
                                                flash_single_slot = Some(*n);
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            });

                            ui.end_row();
                        }
                    });
            });

        if let Some(disk_num) = flash_single_slot {
            self.state.flash_single_device(disk_num);
        }
        if let Some(disk_num) = manual_retry_slot {
            self.state.manual_retry_device(disk_num);
        }
        if let Some(disk_num) = stop_beacon_slot {
            self.state.stop_beacon(disk_num);
        }
    }

    fn render_benchmark_table(&mut self, ui: &mut egui::Ui) {
        let t = self.state.t();
        let colors = self.theme.colors();
        let total_count = self.state.devices.len();
        let active_count = self.state.benchmark_drives.values().filter(|d| d.is_active).count();

        ui.horizontal(|ui| {
            ui.heading(RichText::new(t.tab_benchmark).size(17.0).strong());
            ui.label(
                RichText::new(format!(
                    "({} connected, {} testing)",
                    total_count, active_count
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
                            RichText::new("⚡")
                                .size(36.0)
                                .color(colors.empty_icon_color),
                        );
                        ui.add_space(8.0);
                        ui.label(RichText::new(t.bench_no_devices).size(15.0).weak());
                    });
                });
            return;
        }

        let mut start_bench_single = None;
        let mut stop_bench = false;

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Grid::new("benchmark_grid")
                    .striped(true)
                    .spacing(Vec2::new(14.0, 10.0))
                    .min_col_width(50.0)
                    .show(ui, |ui| {
                        // Headers
                        ui.label(RichText::new(t.bench_col_device).strong().weak());
                        ui.label(RichText::new(t.bench_col_status).strong().weak());
                        ui.label(RichText::new(t.bench_col_write).strong().weak());
                        ui.label(RichText::new(t.bench_col_stability).strong().weak());
                        ui.label(RichText::new(t.bench_col_read).strong().weak());
                        ui.label(RichText::new(t.bench_col_integrity).strong().weak());
                        ui.label(RichText::new("Stability Graph").strong().weak());
                        ui.label(RichText::new(t.bench_col_actions).strong().weak());
                        ui.end_row();

                        for (n, slot) in &self.state.devices {
                            let gb = slot.info.size_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
                            let bench_state = self.state.benchmark_drives.get(n);

                            // Col 1: Device
                            ui.vertical(|ui| {
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new(format!("#{}", n)).strong().color(Color32::from_rgb(100, 200, 255)));
                                    ui.label(RichText::new(format!("{:.2} GB", gb)).strong());
                                });
                                ui.label(RichText::new(&slot.info.product).size(11.5).strong());
                                ui.label(RichText::new(&slot.info.path).monospace().size(10.5).weak());
                            });

                            // Col 2: Status / Progress
                            let phase = bench_state.map(|s| &s.phase).unwrap_or(&BenchmarkPhase::Idle);
                            let progress = bench_state.map(|s| s.progress).unwrap_or(0.0);
                            let is_active = bench_state.map(|s| s.is_active).unwrap_or(false);

                            let (status_text, status_color) = match phase {
                                BenchmarkPhase::Idle => (t.bench_status_idle, Color32::from_rgb(120, 120, 130)),
                                BenchmarkPhase::WipingPartitions => (t.bench_status_wiping, Color32::from_rgb(255, 152, 0)),
                                BenchmarkPhase::Writing => (t.bench_status_writing, Color32::from_rgb(33, 150, 243)),
                                BenchmarkPhase::Flushing => (t.bench_status_flushing, Color32::from_rgb(156, 39, 176)),
                                BenchmarkPhase::Reading => (t.bench_status_reading, Color32::from_rgb(0, 150, 136)),
                                BenchmarkPhase::Done => (t.bench_status_done, Color32::from_rgb(76, 175, 80)),
                                BenchmarkPhase::Failed(_) => (t.bench_status_failed, Color32::from_rgb(244, 67, 54)),
                                BenchmarkPhase::Cancelled => (t.bench_status_cancelled, Color32::from_rgb(255, 179, 0)),
                            };

                            ui.vertical(|ui| {
                                egui::Frame::none()
                                    .fill(status_color.linear_multiply(0.2))
                                    .stroke(Stroke::new(1.0_f32, status_color))
                                    .rounding(Rounding::same(4.0))
                                    .inner_margin(egui::Margin::symmetric(6.0, 2.0))
                                    .show(ui, |ui| {
                                        ui.label(RichText::new(status_text).color(status_color).strong().size(11.5));
                                    });

                                ui.add_space(2.0);
                                if is_active || matches!(phase, BenchmarkPhase::Done) {
                                    ui.add(
                                        egui::ProgressBar::new(progress)
                                            .desired_width(130.0)
                                            .desired_height(12.0)
                                            .animate(is_active)
                                            .text(format!("{:.0}%", progress * 100.0)),
                                    );
                                }
                            });

                            // Col 3: Write Performance
                            let result = bench_state.and_then(|s| s.result.as_ref());
                            ui.vertical(|ui| {
                                if let Some(res) = result {
                                    ui.label(
                                        RichText::new(format!("{:.1} MB/s", res.write_metrics.avg_mbps))
                                            .strong()
                                            .color(Color32::from_rgb(255, 152, 0)),
                                    );
                                    ui.label(
                                        RichText::new(format!(
                                            "min {:.1} | max {:.1}",
                                            res.write_metrics.min_mbps, res.write_metrics.max_mbps
                                        ))
                                        .size(10.5)
                                        .weak(),
                                    );
                                } else if let Some(bs) = bench_state {
                                    if bs.phase == BenchmarkPhase::Writing && bs.current_speed_mbps > 0.0 {
                                        ui.label(
                                            RichText::new(format!("⚡ {:.1} MB/s", bs.current_speed_mbps))
                                                .strong()
                                                .color(Color32::from_rgb(255, 152, 0)),
                                        );
                                    } else {
                                        ui.label(RichText::new("—").weak());
                                    }
                                } else {
                                    ui.label(RichText::new("—").weak());
                                }
                            });

                            // Col 4: Write Stability
                            ui.vertical(|ui| {
                                if let Some(res) = result {
                                    let stab = res.write_metrics.stability_pct;
                                    let stab_color = if stab >= 80.0 {
                                        Color32::from_rgb(76, 175, 80)
                                    } else if stab >= 60.0 {
                                        Color32::from_rgb(255, 193, 7)
                                    } else {
                                        Color32::from_rgb(244, 67, 54)
                                    };

                                    let verdict = match self.state.lang {
                                        mass_usb_flasher::i18n::Lang::Ru => res.write_metrics.verdict_ru(),
                                        mass_usb_flasher::i18n::Lang::En => res.write_metrics.verdict_en(),
                                    };

                                    ui.label(
                                        RichText::new(format!("{:.0}%", stab))
                                            .strong()
                                            .color(stab_color),
                                    );
                                    ui.label(RichText::new(verdict).size(10.5).color(stab_color));
                                } else {
                                    ui.label(RichText::new("—").weak());
                                }
                            });

                            // Col 5: Read Performance & Stability
                            ui.vertical(|ui| {
                                if let Some(res) = result {
                                    ui.label(
                                        RichText::new(format!("{:.1} MB/s", res.read_metrics.avg_mbps))
                                            .strong()
                                            .color(Color32::from_rgb(0, 230, 118)),
                                    );
                                    ui.label(
                                        RichText::new(format!(
                                            "min {:.1} | max {:.1} (stab {:.0}%)",
                                            res.read_metrics.min_mbps, res.read_metrics.max_mbps, res.read_metrics.stability_pct
                                        ))
                                        .size(10.5)
                                        .weak(),
                                    );
                                } else if let Some(bs) = bench_state {
                                    if bs.phase == BenchmarkPhase::Reading && bs.current_speed_mbps > 0.0 {
                                        ui.label(
                                            RichText::new(format!("⚡ {:.1} MB/s", bs.current_speed_mbps))
                                                .strong()
                                                .color(Color32::from_rgb(0, 230, 118)),
                                        );
                                    } else {
                                        ui.label(RichText::new("—").weak());
                                    }
                                } else {
                                    ui.label(RichText::new("—").weak());
                                }
                            });

                            // Col 6: Integrity
                            ui.vertical(|ui| {
                                if let Some(res) = result {
                                    if res.verify_errors == 0 {
                                        ui.label(
                                            RichText::new(t.bench_integrity_ok)
                                                .strong()
                                                .color(Color32::from_rgb(76, 175, 80))
                                                .size(11.5),
                                        );
                                    } else {
                                        ui.label(
                                            RichText::new(format!("❌ {} ERRORS", res.verify_errors))
                                                .strong()
                                                .color(Color32::from_rgb(244, 67, 54))
                                                .size(11.5),
                                        );
                                    }
                                    if res.partitions_wiped {
                                        ui.label(RichText::new("MBR/GPT wiped ✔").weak().size(10.0));
                                    }
                                    if res.controller_disconnects > 0 {
                                        let disc_msg = match self.state.lang {
                                            mass_usb_flasher::i18n::Lang::Ru => format!("⚠️ {} сбоев перегрева (восстановлен)", res.controller_disconnects),
                                            mass_usb_flasher::i18n::Lang::En => format!("⚠️ {} thermal reset(s) (recovered)", res.controller_disconnects),
                                        };
                                        ui.label(RichText::new(disc_msg).color(Color32::from_rgb(255, 179, 0)).size(10.0));
                                    }
                                } else {
                                    ui.label(RichText::new("—").weak());
                                }
                            });

                            // Col 7: Speed Stability Sparkline Graph
                            let empty_samples = Vec::new();
                            let write_samples = result
                                .map(|r| &r.write_samples)
                                .or_else(|| bench_state.map(|s| &s.live_write_samples))
                                .unwrap_or(&empty_samples);
                            let read_samples = result
                                .map(|r| &r.read_samples)
                                .or_else(|| bench_state.map(|s| &s.live_read_samples))
                                .unwrap_or(&empty_samples);

                            ui.vertical(|ui| {
                                draw_speed_sparkline(ui, write_samples, read_samples, 150.0, 34.0);
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new("■ Write").size(9.5).color(Color32::from_rgb(255, 152, 0)));
                                    ui.label(RichText::new("■ Read").size(9.5).color(Color32::from_rgb(0, 230, 118)));
                                });
                            });

                            // Col 8: Actions
                            ui.vertical(|ui| {
                                if is_active {
                                    if ui.small_button(RichText::new(t.bench_btn_stop).color(Color32::from_rgb(255, 100, 100))).clicked() {
                                        stop_bench = true;
                                    }
                                } else {
                                    let btn = egui::Button::new(
                                        RichText::new(t.bench_btn_single)
                                            .strong()
                                            .color(Color32::from_rgb(100, 200, 255)),
                                    );
                                    if ui.add_enabled(!self.state.benchmark_active, btn).on_hover_text("Wipes partition table and runs benchmark on this drive").clicked() {
                                        start_bench_single = Some(*n);
                                    }
                                }
                            });

                            ui.end_row();
                        }
                    });
            });

        if let Some(disk_num) = start_bench_single {
            self.state.start_benchmark_single(disk_num);
        }
        if stop_bench {
            self.state.stop_benchmark();
        }
    }

    fn render_benchmark_bottom_bar(&mut self, ui: &mut egui::Ui) {
        let t = self.state.t();

        // Banner if benchmark is active or finished
        if self.state.benchmark_active {
            egui::Frame::none()
                .fill(Color32::from_rgb(28, 55, 90))
                .rounding(Rounding::same(6.0))
                .inner_margin(egui::Margin::symmetric(14.0, 7.0))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("⚡ FLASH DRIVE BENCHMARK IN PROGRESS… TESTING WRITE & READ STABILITY")
                                .strong()
                                .size(13.5)
                                .color(Color32::from_rgb(100, 200, 255)),
                        );
                    });
                });
            ui.add_space(6.0);
        } else if self.state.benchmark_drives.values().any(|d| d.result.is_some()) {
            egui::Frame::none()
                .fill(Color32::from_rgb(34, 90, 50))
                .rounding(Rounding::same(6.0))
                .inner_margin(egui::Margin::symmetric(14.0, 7.0))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(t.bench_summary_finished)
                                .strong()
                                .size(13.5)
                                .color(Color32::WHITE),
                        );
                    });
                });
            ui.add_space(6.0);
        }

        ui.horizontal(|ui| {
            if self.state.benchmark_active {
                let stop_btn = egui::Button::new(
                    RichText::new(t.bench_btn_stop)
                        .size(15.0)
                        .strong()
                        .color(Color32::WHITE),
                )
                .fill(Color32::from_rgb(211, 47, 47))
                .min_size(Vec2::new(180.0, 36.0));

                if ui.add(stop_btn).clicked() {
                    self.state.stop_benchmark();
                }
            } else {
                let can_start = !self.state.devices.is_empty() && self.state.status != AppStatus::Running;
                let start_btn = egui::Button::new(
                    RichText::new(t.bench_btn_run_all)
                        .size(15.0)
                        .strong()
                        .color(if can_start { Color32::WHITE } else { Color32::from_rgb(140, 140, 140) }),
                )
                .fill(if can_start { Color32::from_rgb(46, 125, 50) } else { Color32::from_rgb(45, 50, 45) })
                .min_size(Vec2::new(180.0, 36.0));

                if ui.add_enabled(can_start, start_btn).clicked() {
                    self.state.start_benchmark_all();
                }
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

                ui.hyperlink_to(
                    RichText::new(WATERMARK).weak().size(12.0),
                    WATERMARK_URL,
                );
            });
        });
    }

    fn render_bottom_bar(&mut self, ui: &mut egui::Ui) {
        let t = self.state.t();

        // 1. OVERALL PROGRESS & ANIMATED COMPLETION BANNER
        if self.state.status == AppStatus::Finished {
            let time = ui.input(|i| i.time);
            ui.ctx().request_repaint_after(Duration::from_millis(50));
            let pulse = ((time * 4.0).sin() * 0.5 + 0.5) as f32;

            let r = (34.0 * (1.0 - pulse) + 255.0 * pulse) as u8;
            let g = (180.0 * (1.0 - pulse) + 215.0 * pulse) as u8;
            let b = (76.0 * (1.0 - pulse) + 0.0 * pulse) as u8;
            let banner_color = Color32::from_rgb(r, g, b);

            egui::Frame::none()
                .fill(banner_color)
                .rounding(Rounding::same(6.0))
                .inner_margin(egui::Margin::symmetric(14.0, 7.0))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("🎉 FLASHING COMPLETE! ALL TARGET FLASH DRIVES READY ✔")
                                .strong()
                                .size(15.0)
                                .color(Color32::BLACK),
                        );
                        if let Some(elapsed) = self.state.elapsed_secs() {
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.label(
                                    RichText::new(format!("Elapsed: {}", format_elapsed(elapsed)))
                                        .strong()
                                        .color(Color32::BLACK),
                                );
                            });
                        }
                    });
                });
            ui.add_space(6.0);
        } else if self.state.status == AppStatus::Running {
            let overall_pct = self.state.overall_progress_fraction();
            let total_target = self.state.total_target_bytes();
            let written = self.state.total_bytes_written();
            let remaining = total_target.saturating_sub(written);

            let speed_str = self
                .state
                .per_device_speed_mbps()
                .map(|s| format!("{:.1} MB/s/drive", s))
                .unwrap_or_else(|| "calculating…".to_string());

            let eta_str = if let Some(speed_mbps) = self.state.per_device_speed_mbps() {
                let active_count = self.state.active_slots().len().max(1);
                let total_speed_bytes = speed_mbps * 1_048_576.0 * active_count as f64;
                if total_speed_bytes > 0.0 {
                    let secs = remaining as f64 / total_speed_bytes;
                    format_elapsed(secs)
                } else {
                    "—".to_string()
                }
            } else {
                "calculating…".to_string()
            };

            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("📊 {}:", t.gui_overall_progress)).strong().size(13.0));
                ui.label(
                    RichText::new(format!("{:.1}%", overall_pct * 100.0))
                        .strong()
                        .size(13.0)
                        .color(Color32::from_rgb(100, 200, 255)),
                );
                ui.label(RichText::new(format!("({:.2} / {:.2} GB)", written as f64 / 1e9, total_target as f64 / 1e9)).weak());
                ui.separator();
                ui.label(RichText::new(format!("⚡ {}", speed_str)).color(Color32::from_rgb(33, 150, 243)));
                ui.separator();
                ui.label(RichText::new(format!("⏳ ETA: {}", eta_str)).color(Color32::from_rgb(255, 193, 7)));
                if let Some(elapsed) = self.state.elapsed_secs() {
                    ui.separator();
                    ui.label(RichText::new(format!("Elapsed: {}", format_elapsed(elapsed))).weak());
                }
            });
            ui.add_space(2.0);
            ui.add(
                egui::ProgressBar::new(overall_pct)
                    .desired_height(16.0)
                    .animate(true)
                    .text(format!("{:>5.1}%", overall_pct * 100.0)),
            );
            ui.add_space(6.0);
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
                        }
                    }
                    if ui.button(t.cat_hint_f).clicked() {
                        if let Some(folder) = mass_usb_flasher::picker::pick_image_folder() {
                            self.state.apply_import_folder(folder);
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

fn draw_speed_sparkline(
    ui: &mut egui::Ui,
    write_samples: &[f32],
    read_samples: &[f32],
    width: f32,
    height: f32,
) {
    let desired_size = Vec2::new(width, height);
    let (rect, _resp) = ui.allocate_exact_size(desired_size, egui::Sense::hover());
    let painter = ui.painter_at(rect);

    // Фоновая плашка
    painter.rect_filled(rect, Rounding::same(3.0), Color32::from_rgb(20, 22, 28));
    painter.rect_stroke(
        rect,
        Rounding::same(3.0),
        Stroke::new(1.0_f32, Color32::from_rgb(45, 48, 58)),
    );

    if write_samples.is_empty() && read_samples.is_empty() {
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            "No data",
            egui::FontId::proportional(10.0),
            Color32::from_rgb(90, 95, 110),
        );
        return;
    }

    let mut max_speed = 10.0f32;
    for &s in write_samples.iter().chain(read_samples.iter()) {
        if s > max_speed {
            max_speed = s;
        }
    }

    // Линия скорости записи (оранжевая)
    if write_samples.len() >= 2 {
        let n = write_samples.len();
        let points: Vec<egui::Pos2> = write_samples
            .iter()
            .enumerate()
            .map(|(i, &v)| {
                let x = rect.left() + 2.0 + (i as f32 / (n - 1) as f32) * (rect.width() - 4.0);
                let norm = (v / max_speed).clamp(0.0, 1.0);
                let y = rect.bottom() - 2.0 - norm * (rect.height() - 4.0);
                egui::pos2(x, y)
            })
            .collect();

        for w in points.windows(2) {
            painter.line_segment([w[0], w[1]], Stroke::new(1.5_f32, Color32::from_rgb(255, 152, 0)));
        }
    }

    // Линия скорости чтения (зеленая/бирюзовая)
    if read_samples.len() >= 2 {
        let n = read_samples.len();
        let points: Vec<egui::Pos2> = read_samples
            .iter()
            .enumerate()
            .map(|(i, &v)| {
                let x = rect.left() + 2.0 + (i as f32 / (n - 1) as f32) * (rect.width() - 4.0);
                let norm = (v / max_speed).clamp(0.0, 1.0);
                let y = rect.bottom() - 2.0 - norm * (rect.height() - 4.0);
                egui::pos2(x, y)
            })
            .collect();

        for w in points.windows(2) {
            painter.line_segment([w[0], w[1]], Stroke::new(1.5_f32, Color32::from_rgb(0, 230, 118)));
        }
    }
}
