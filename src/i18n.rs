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
}

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const WATERMARK: &str = "Vibe-coded by TG:@delyoself";
pub const WATERMARK_URL: &str = "https://t.me/delyoself";
