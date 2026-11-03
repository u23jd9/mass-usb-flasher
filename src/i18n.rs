//! Локализация интерфейса.
//! Английский — по умолчанию, русский — переключается клавишей L.

macro_rules! i18n {
    (
        $(
            $name:ident = $en:literal / $ru:literal
        ),* $(,)?
    ) => {
        pub struct T {
            $(pub $name: &'static str,)*
        }
        pub const EN: T = T { $($name: $en,)* };
        pub const RU: T = T { $($name: $ru,)* };
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    Ru,
}

impl Default for Lang {
    fn default() -> Self {
        Lang::En
    }
}

impl Lang {
    pub fn toggle(self) -> Self {
        match self {
            Lang::En => Lang::Ru,
            Lang::Ru => Lang::En,
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "EN",
            Lang::Ru => "RU",
        }
    }

    pub fn t(self) -> &'static T {
        match self {
            Lang::En => &EN,
            Lang::Ru => &RU,
        }
    }
}

i18n! {
    // Статусы (верхний заголовок)
    status_idle = "Idle" / "Ожидание",
    status_waiting = "Waiting for devices" / "Ожидание устройств",
    status_ready = "Ready" / "Готово",
    status_running = "Running" / "Работа",
    status_finished = "Finished" / "Завершено",

    // Статусы слотов
    slot_waiting = "Waiting" / "Ожидание",
    slot_ready = "Ready" / "Готово",
    slot_writing = "Writing" / "Запись",
    slot_verifying = "Verifying" / "Проверка",
    slot_formatting = "Formatting" / "Формат",
    slot_done = "Done" / "Готово",
    slot_failed = "FAILED" / "ОШИБКА",
    slot_retrying = "Retrying" / "Повтор",

    // Лейблы header
    lbl_status = "Status:" / "Статус:",
    lbl_target = "Target:" / "Цель:",
    lbl_min = "Min:" / "Мин:",
    lbl_max = "Max:" / "Макс:",
    lbl_image = "Image:" / "Образ:",
    lbl_sha256 = "SHA256:" / "SHA256:",
    lbl_elapsed = "Elapsed:" / "Время:",
    lbl_speed = "Speed:" / "Скорость:",
    lbl_written = "Written:" / "Записано:",
    speed_total = "MB/s total" / "МБ/с суммарно",
    speed_per_device = "MB/s per device" / "МБ/с на устройство",

    // Строка образа
    image_none = "--- not selected (press I) ---" / "--- не выбран (нажми I) ---",
    image_no_name = "(unnamed)" / "(без названия)",
    sha_computing = "computing…" / "вычисляется…",
    sha_starting = "starting…" / "запускается…",
    target_extras_fmt = " (found {}, extras untouched)" / " (найдено {}, лишние не трогаем)",

    // Footer & Actions
    fk_open = "Open" / "Образ",
    fk_start = "Start" / "Старт",
    fk_help = "Help" / "Справка",
    fk_target = "Target" / "Цель",
    fk_lang = "Lang" / "Язык",
    fk_quit = "Quit" / "Выход",
    fk_reset = "Ready" / "Сброс",
    status_finished_hint = "Batch finished! Press [R/Enter] to switch back to Ready/Idle" / "Партия завершена! Нажмите [R/Enter] для сброса в Готово/Ожидание",

    // Заголовки таблицы
    th_id = "ID" / "ID",
    th_path = "Path" / "Путь",
    th_size = "Size (GB)" / "Размер (ГБ)",
    th_product = "Product" / "Модель",
    th_serial = "Serial" / "Серийный",
    th_status = "Status" / "Статус",
    th_progress = "Progress" / "Прогресс",

    // Заголовки блоков
    title_app = "Mass USB Flasher" / "Mass USB Flasher",
    title_devices_fmt = " Devices: {} (ready: {}, need: {}) " / " Устройства: {} (готово: {}, нужно: {}) ",
    title_catalog_fmt = " Image Catalog ({}) " / " Каталог образов ({}) ",
    title_new_image = " New Image " / " Новый образ ",
    title_rename = " Rename " / " Переименование ",
    title_help = " Help " / " Справка ",

    // Каталог
    catalog_empty = "Catalog is empty. [A] add, [F] import folder." / "Каталог пуст. [A] добавить, [F] импорт папки.",
    cat_hint_enter = "Select" / "Выбрать",
    cat_hint_a = "Add" / "Добавить",
    cat_hint_o = "Open once" / "Открыть разово",
    cat_hint_f = "Import folder" / "Импорт папки",
    cat_hint_r = "Rename" / "Переименовать",
    cat_hint_d = "Delete" / "Удалить",
    cat_hint_esc = "Back" / "Назад",

    // Модалки
    prompt_work_name = "Work name" / "Рабочее имя",
    prompt_new_name = "New name" / "Новое имя",
    prompt_hint = "Enter — ok, Esc — cancel" / "Enter — ок, Esc — отмена",
    modal_size = "Size:" / "Размер:",

    // Импорт
    imp_scanning_fmt = "scanning, files: {}" / "сканирование, файлов: {}",
    imp_hashing_fmt = "[{}/{}]" / "[{}/{}]",
    imp_ready = "ready:" / "готов:",
    imp_done_fmt = "import complete: {}" / "импорт завершён: {}",
    imp_error_fmt = "error: {}" / "ошибка: {}",

    // Хоткеи в футере
    hotkeys_target_hint = "[+/-]" / "[+/-]",

    // GUI специфичные
    gui_btn_start = "▶ Start Flashing" / "▶ Запустить запись",
    gui_btn_reset = "🔄 Ready for Next Batch" / "🔄 Готово к новой партии (Сброс)",
    gui_btn_select = "📂 Select Image..." / "📂 Выбрать образ...",
    gui_btn_catalog = "📚 Catalog" / "📚 Каталог образов",
    gui_btn_open_once = "⚡ Open Once..." / "⚡ Открыть разово...",
    gui_title_devices = "Connected USB Drives" / "Подключённые USB-накопители",
    gui_title_log = "Activity Log" / "Журнал событий",
    gui_no_devices = "No USB devices detected. Insert USB drives to start." / "USB-накопители не обнаружены. Вставьте флешки для записи.",
    gui_waiting_devices = "Waiting for required number of drives" / "Ожидание достаточного количества устройств",
    gui_all_done = "🎉 All target flash drives written successfully!" / "🎉 Все целевые флешки успешно записаны!",
    gui_theme_dark = "🌙 Dark" / "🌙 Тёмная",
    gui_theme_light = "☀️ Light" / "☀️ Светлая",
    gui_file_picker_title = "Browse Files" / "Обзор файлов",
    gui_btn_select_dir = "✔ Select This Folder" / "✔ Выбрать эту папку",
    gui_up_dir = "⬆ Up" / "⬆ Вверх",
    gui_btn_retry = "🔄 Retry" / "🔄 Повторить",
    gui_log_clear = "🗑 Clear" / "🗑 Очистить",
    gui_log_autoscroll = "Auto-scroll" / "Автопрокрутка",
    gui_sidebar_title = "Image Library" / "Библиотека образов",
    gui_sidebar_toggle = "📁 Library" / "📁 Библиотека",
    gui_btn_flash_single = "⚡ Flash" / "⚡ Записать",
    gui_menu_flash_single = "⚡ Flash only this drive" / "⚡ Прошить только эту флешку",
    gui_btn_stop_beacon = "⏹ Stop Beacon" / "⏹ Стоп маяк",
    gui_autotarget_btn = "Auto" / "Авто",
    gui_overall_progress = "Overall Progress" / "Общий прогресс",

    // Benchmark Tab
    tab_flashing = "Flashing" / "Запись образов",
    tab_benchmark = "Benchmark" / "Бенчмарк флешек",
    bench_title = "USB Flash Drive Benchmark & Stability Tester" / "Бенчмарк и проверка стабильности флешек",
    bench_subtitle = "Tests sequential write & read, measures speed stability over time, and wipes partition table before testing" / "Тестирует запись и чтение, измеряет стабильность скорости и затирает таблицу разделов перед стартом",
    bench_btn_run_all = "▶ Run Benchmark" / "▶ Запустить бенчмарк",
    bench_btn_stop = "⏹ Stop Benchmark" / "⏹ Остановить бенчмарк",
    bench_btn_single = "⚡ Benchmark" / "⚡ Бенчмарк",
    bench_test_size = "Test Size:" / "Объём теста:",
    bench_lbl_mode = "Mode:" / "Режим:",
    bench_mode_standard = "Standard (Continuous)" / "Обычный (Сплошной)",
    bench_mode_detailed = "Detailed (Thermal Stress & Recovery)" / "Детальный (Стресс-тест и перегрев)",
    bench_lbl_concurrency = "Queue:" / "Очередь:",
    bench_concurrency_seq = "Sequential (Safe for USB Hubs)" / "Последовательно (Безопасно для хабов)",
    bench_concurrency_par = "Parallel (Simultaneous)" / "Параллельно (Все сразу)",
    bench_wipe_partitions = "Wipe partition table before test (MBR/GPT)" / "Затереть таблицу разделов перед тестом (MBR/GPT)",
    bench_status_idle = "Ready to Benchmark" / "Готово к бенчмарку",
    bench_status_wiping = "Wiping partition table…" / "Затирание разделов…",
    bench_status_writing = "Writing test…" / "Тест записи…",
    bench_status_flushing = "Flushing buffer…" / "Сброс буфера…",
    bench_status_reading = "Reading & verifying…" / "Тест чтения и проверка…",
    bench_status_done = "Benchmark Finished" / "Бенчмарк завершён",
    bench_status_failed = "Benchmark Failed" / "Ошибка бенчмарка",
    bench_status_cancelled = "Cancelled" / "Отменено",
    bench_col_device = "Device / Model" / "Устройство / Модель",
    bench_col_status = "Status / Progress" / "Статус / Прогресс",
    bench_col_write = "Write Performance" / "Скорость записи",
    bench_col_read = "Read Performance" / "Скорость чтения",
    bench_col_stability = "Speed Stability & Graph" / "Стабильность и график",
    bench_col_integrity = "Integrity" / "Целостность",
    bench_integrity_ok = "✔ 100% OK" / "✔ 100% ОК",
    bench_col_actions = "Actions" / "Действия",
    bench_lbl_avg = "Avg:" / "Сред:",
    bench_lbl_min = "Min:" / "Мин:",
    bench_lbl_max = "Max:" / "Макс:",
    bench_lbl_stability = "Stability:" / "Стабильность:",
    bench_no_devices = "No USB flash drives found to benchmark. Insert USB drives." / "Нет обнаруженных флешек для бенчмарка. Подключите USB-накопители.",
    bench_summary_finished = "🎉 Benchmark completed for all drives!" / "🎉 Бенчмарк всех флешек успешно завершён!",
    bench_summary_running = "⚡ Benchmarking drives in progress…" / "⚡ Выполняется тестирование флешек…",
}

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const WATERMARK: &str = "Vibe-coded by TG:@delyoself";
pub const WATERMARK_URL: &str = "https://t.me/delyoself";
