use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::thread;
use std::time::Instant;

use crate::catalog::{spawn_folder_import, unix_now, Catalog, ImageEntry, ImportProgress};
use crate::device::watcher::{DeviceEvent, WatcherHandle};
use crate::device::DiskInfo;
use crate::engine::writer::{start_write, WriteEvent};
use crate::i18n::{Lang, T};
use crate::image::{compute_sha256_async, HashProgress};

pub enum PrepareResult {
    Log(String),
    Ready {
        write_rx: Receiver<WriteEvent>,
        opened_numbers: Vec<u32>,
        files_count: usize,
    },
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotStatus {
    Waiting,
    Ready,
    Writing,
    Verifying,
    Formatting,
    Done,
    Failed,
}

#[derive(Debug, Clone)]
pub struct DeviceSlot {
    pub info: DiskInfo,
    pub status: SlotStatus,
    pub progress: f32,
    pub bytes_written: u64,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppStatus {
    Idle,
    WaitingForDevices,
    Ready,
    Running,
    Finished,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Main,
    ImageSelect,
    AddImageName,
    RenameImage,
    Help,
}

pub struct HashJob {
    pub rx: Receiver<HashProgress>,
    pub save_to_catalog: bool,
}

const TARGET_MIN: usize = 1;
const TARGET_MAX: usize = 64;

pub struct AppState {
    pub image_path: Option<PathBuf>,
    pub image_size: u64,
    pub image_sha256: Option<String>,
    pub image_hash_progress: Option<(u64, u64)>,
    pub image_work_name: Option<String>,
    pub hash_job: Option<HashJob>,

    pub catalog: Catalog,
    pub catalog_path: PathBuf,

    pub screen: Screen,
    pub image_select_index: usize,
    pub pending_image_path: Option<PathBuf>,
    pub pending_image_size: u64,
    pub input_buffer: String,
    pub rename_target_index: Option<usize>,

    pub import_job: Option<Receiver<ImportProgress>>,
    pub import_progress: Option<ImportProgress>,

    pub target_count: usize,
    pub min_size_bytes: u64,
    pub max_size_bytes: u64,

    pub devices: BTreeMap<u32, DeviceSlot>,
    pub status: AppStatus,

    pub prepare_rx: Option<Receiver<PrepareResult>>,
    pub write_rx: Option<Receiver<WriteEvent>>,
    pub write_total_bytes: u64,
    pub write_started_at: Option<Instant>,

    pub lang: Lang,

    pub log: Vec<String>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            image_path: None,
            image_size: 0,
            image_sha256: None,
            image_hash_progress: None,
            image_work_name: None,
            hash_job: None,

            catalog: Catalog::default(),
            catalog_path: crate::catalog::default_catalog_path(),

            screen: Screen::Main,
            image_select_index: 0,
            pending_image_path: None,
            pending_image_size: 0,
            input_buffer: String::new(),
            rename_target_index: None,

            import_job: None,
            import_progress: None,

            target_count: 2,
            min_size_bytes: 4 * 1024 * 1024 * 1024,
            max_size_bytes: 128 * 1024 * 1024 * 1024,

            devices: BTreeMap::new(),
            status: AppStatus::Idle,

            prepare_rx: None,
            write_rx: None,
            write_total_bytes: 0,
            write_started_at: None,

            lang: Lang::default(),

            log: Vec::new(),
        }
    }
}

impl AppState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn t(&self) -> &'static T {
        self.lang.t()
    }

    pub fn push_log(&mut self, line: impl Into<String>) {
        let line = line.into();
        let ts = timestamp();
        self.log.push(format!("{} {}", ts, line));
        if self.log.len() > 1000 {
            self.log.drain(0..self.log.len() - 1000);
        }
    }

    // --- Устройства ---

    pub fn apply_device_event(&mut self, ev: DeviceEvent) {
        match ev {
            DeviceEvent::Arrived(info) => self.on_device_arrived(info),
            DeviceEvent::Removed { number, path } => {
                self.push_log(format!("[-] #{} {}", number, path));
                self.devices.remove(&number);
                self.recompute_status();
            }
        }
    }

    fn on_device_arrived(&mut self, info: DiskInfo) {
        let n = info.number;
        let gb = info.size_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
        self.push_log(format!(
            "[+] #{} {} ({:.2} GB) SN={}",
            n, info.product, gb, info.serial
        ));
        self.devices.insert(
            n,
            DeviceSlot {
                info,
                status: SlotStatus::Waiting,
                progress: 0.0,
                bytes_written: 0,
                message: String::new(),
            },
        );
        self.recompute_status();
    }

    pub fn recompute_status(&mut self) {
        if matches!(self.status, AppStatus::Running | AppStatus::Finished) {
            return;
        }

        let have = self.devices.len();
        let target = self.target_count;

        self.status = if have == 0 {
            AppStatus::Idle
        } else if have < target {
            AppStatus::WaitingForDevices
        } else {
            AppStatus::Ready
        };

        let mut sorted_numbers: Vec<u32> = self.devices.keys().copied().collect();
        sorted_numbers.sort_unstable();
        let ready_numbers: std::collections::BTreeSet<u32> =
            sorted_numbers.into_iter().take(target).collect();

        for (n, slot) in self.devices.iter_mut() {
            if matches!(slot.status, SlotStatus::Waiting | SlotStatus::Ready) {
                slot.status = if ready_numbers.contains(n) {
                    SlotStatus::Ready
                } else {
                    SlotStatus::Waiting
                };
            }
        }
    }

    pub fn ready_device_numbers(&self) -> Vec<u32> {
        let mut v: Vec<u32> = self
            .devices
            .iter()
            .filter(|(_, s)| s.status == SlotStatus::Ready)
            .map(|(n, _)| *n)
            .collect();
        v.sort_unstable();
        v
    }

    pub fn ready_count(&self) -> usize {
        self.devices
            .values()
            .filter(|s| s.status == SlotStatus::Ready)
            .count()
    }

    pub fn active_or_ready_count(&self) -> usize {
        self.devices
            .values()
            .filter(|s| {
                matches!(
                    s.status,
                    SlotStatus::Ready
                        | SlotStatus::Writing
                        | SlotStatus::Verifying
                        | SlotStatus::Formatting
                        | SlotStatus::Done
                        | SlotStatus::Failed
                )
            })
            .count()
    }

    pub fn set_target_count(&mut self, n: usize) {
        self.target_count = n.clamp(TARGET_MIN, TARGET_MAX);
        self.recompute_status();
    }

    pub fn increase_target(&mut self) {
        self.set_target_count(self.target_count.saturating_add(1));
    }

    pub fn decrease_target(&mut self) {
        self.set_target_count(self.target_count.saturating_sub(1));
    }

    pub fn update_progress_by_index(&mut self, index: usize, bytes_written: u64, total: u64) {
        let Some((_, slot)) = self.devices.iter_mut().nth(index) else {
            return;
        };
        slot.bytes_written = bytes_written;
        slot.progress = if total > 0 {
            (bytes_written as f32 / total as f32).min(1.0)
        } else {
            0.0
        };
    }

    pub fn mark_completed_by_index(&mut self, index: usize) {
        let total = self.write_total_bytes;
        if let Some((_, slot)) = self.devices.iter_mut().nth(index) {
            slot.status = SlotStatus::Done;
            slot.progress = 1.0;
            slot.bytes_written = total.max(slot.bytes_written);
            slot.message.clear();
        }
    }

    pub fn mark_failed_by_index(&mut self, index: usize, message: String) {
        if let Some((_, slot)) = self.devices.iter_mut().nth(index) {
            slot.status = SlotStatus::Failed;
            slot.message = message;
        }
    }

    pub fn mark_slots_writing(&mut self, opened_numbers: &[u32]) {
        let opened: std::collections::BTreeSet<u32> = opened_numbers.iter().copied().collect();
        for (n, slot) in self.devices.iter_mut() {
            if opened.contains(n) {
                slot.status = SlotStatus::Writing;
                slot.progress = 0.0;
                slot.bytes_written = 0;
                slot.message.clear();
            } else if slot.status != SlotStatus::Failed {
                slot.status = SlotStatus::Failed;
                slot.message = "not opened for write".to_string();
            }
        }
    }

    pub fn rollback_slots_to_ready(&mut self) {
        for slot in self.devices.values_mut() {
            if slot.status != SlotStatus::Failed {
                slot.status = SlotStatus::Ready;
            }
        }
    }

    /// Сброс после завершения записи обратно в состояние Idle/Ready.
    /// Очищает прогресс слотов, обнуляет счётчики записи и пересчитывает статус.
    pub fn reset_to_idle(&mut self) {
        self.status = AppStatus::Idle;
        self.write_total_bytes = 0;
        self.write_started_at = None;
        self.write_rx = None;
        self.prepare_rx = None;

        for slot in self.devices.values_mut() {
            slot.status = SlotStatus::Waiting;
            slot.progress = 0.0;
            slot.bytes_written = 0;
            slot.message.clear();
        }

        self.recompute_status();

        let state_name = match self.status {
            AppStatus::Ready => "Ready",
            AppStatus::WaitingForDevices => "Waiting",
            AppStatus::Idle => "Idle",
            _ => "Ready",
        };

        self.push_log(format!(
            "=== Reset to {} === Devices: {}, Target: {}",
            state_name,
            self.devices.len(),
            self.target_count
        ));
    }

    // --- Метрики ---

    pub fn elapsed_secs(&self) -> Option<f64> {
        self.write_started_at.map(|t| t.elapsed().as_secs_f64())
    }

    fn active_slots(&self) -> Vec<&DeviceSlot> {
        self.devices
            .values()
            .filter(|s| matches!(s.status, SlotStatus::Writing | SlotStatus::Verifying))
            .collect()
    }

    pub fn average_bytes_written(&self) -> u64 {
        let active = self.active_slots();
        if active.is_empty() {
            return 0;
        }
        let sum: u64 = active.iter().map(|s| s.bytes_written).sum();
        sum / active.len() as u64
    }

    pub fn slowest_bytes_written(&self) -> u64 {
        self.active_slots()
            .iter()
            .map(|s| s.bytes_written)
            .min()
            .unwrap_or(0)
    }

    pub fn per_device_speed_mbps(&self) -> Option<f64> {
        let elapsed = self.elapsed_secs()?;
        if elapsed <= 0.5 {
            return None;
        }
        let avg = self.average_bytes_written();
        if avg == 0 {
            return None;
        }
        Some(avg as f64 / 1_048_576.0 / elapsed)
    }

    #[allow(dead_code)]
    pub fn total_bytes_written(&self) -> u64 {
        self.devices.values().map(|s| s.bytes_written).sum()
    }

    pub fn select_image_from_catalog(&mut self, index: usize) -> Result<(), String> {
        let entry = self
            .catalog
            .images
            .get(index)
            .ok_or_else(|| "index out of range".to_string())?
            .clone();

        if !entry.path.exists() {
            return Err(format!("file not found: {}", entry.path.display()));
        }

        self.image_path = Some(entry.path);
        self.image_size = entry.size_bytes;
        self.image_work_name = Some(entry.work_name);
        self.image_sha256 = Some(entry.sha256);
        self.image_hash_progress = None;
        self.hash_job = None;
        Ok(())
    }

    pub fn find_catalog_entry_by_path(
        &self,
        path: &std::path::Path,
    ) -> Option<&crate::catalog::ImageEntry> {
        self.catalog.images.iter().find(|e| e.path == path)
    }

    pub fn rename_in_catalog(&mut self, old_name: &str, new_name: &str) -> Result<(), String> {
        self.catalog.rename(old_name, new_name)?;
        if self.image_work_name.as_deref() == Some(old_name) {
            self.image_work_name = Some(new_name.to_string());
        }
        Ok(())
    }

    // --- Обработка фоновых очередей (Drain) ---

    pub fn drain_device_events(&mut self, handle: &WatcherHandle) {
        while let Ok(ev) = handle.events.try_recv() {
            self.apply_device_event(ev);
        }
    }

    pub fn drain_hash_job(&mut self) {
        let Some(job) = self.hash_job.take() else {
            return;
        };
        let mut keep = true;

        loop {
            match job.rx.try_recv() {
                Ok(HashProgress::Progress {
                    bytes_done,
                    bytes_total,
                }) => {
                    self.image_hash_progress = Some((bytes_done, bytes_total));
                }
                Ok(HashProgress::Done { size_bytes, sha256 }) => {
                    self.image_sha256 = Some(sha256.clone());
                    self.image_hash_progress = None;
                    self.push_log(format!("SHA-256: {}", sha256));
                    if job.save_to_catalog {
                        self.save_current_image_to_catalog(size_bytes, sha256);
                    }
                    keep = false;
                    break;
                }
                Ok(HashProgress::Error(e)) => {
                    self.push_log(format!("[!] Image read error: {}", e));
                    self.image_hash_progress = None;
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
            self.hash_job = Some(job);
        }
    }

    pub fn drain_import_job(&mut self) {
        let Some(rx) = self.import_job.take() else {
            return;
        };
        let mut keep = true;

        loop {
            match rx.try_recv() {
                Ok(p @ ImportProgress::Scanning { total }) => {
                    self.push_log(format!("[import] scanning, files: {}", total));
                    self.import_progress = Some(p);
                }
                Ok(p @ ImportProgress::HashingFile { .. }) => {
                    self.import_progress = Some(p);
                }
                Ok(ImportProgress::FileDone { name }) => {
                    self.push_log(format!("[import] {}", name));
                }
                Ok(ImportProgress::Done { imported, entries }) => {
                    self.push_log(format!("[import] done, imported: {}", imported));
                    for e in entries {
                        self.catalog.upsert(e);
                    }
                    match self.catalog.save(&self.catalog_path) {
                        Ok(()) => self.push_log("Catalog updated"),
                        Err(e) => self.push_log(format!("[!] Catalog: {}", e)),
                    }
                    self.import_progress = None;
                    keep = false;
                    break;
                }
                Ok(ImportProgress::Error(e)) => {
                    self.push_log(format!("[!] Import: {}", e));
                    self.import_progress = None;
                    keep = false;
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.import_progress = None;
                    keep = false;
                    break;
                }
            }
        }

        if keep {
            self.import_job = Some(rx);
        }
    }

    pub fn drain_prepare(&mut self) {
        let Some(rx) = self.prepare_rx.take() else {
            return;
        };
        let mut keep = true;

        loop {
            match rx.try_recv() {
                Ok(PrepareResult::Log(msg)) => self.push_log(msg),
                Ok(PrepareResult::Ready {
                    write_rx,
                    opened_numbers,
                    files_count,
                }) => {
                    self.mark_slots_writing(&opened_numbers);
                    self.write_started_at = Some(Instant::now());
                    self.push_log(format!("=== Writing to {} devices ===", files_count));
                    self.write_rx = Some(write_rx);
                    keep = false;
                    break;
                }
                Ok(PrepareResult::Failed(e)) => {
                    self.push_log(format!("[!] Prepare failed: {}", e));
                    self.status = AppStatus::Ready;
                    self.write_started_at = None;
                    self.rollback_slots_to_ready();
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
            self.prepare_rx = Some(rx);
        }
    }

    pub fn drain_write(&mut self) {
        let Some(rx) = self.write_rx.take() else {
            return;
        };
        let mut keep = true;

        loop {
            match rx.try_recv() {
                Ok(WriteEvent::Progress { id, bytes_written }) => {
                    self.update_progress_by_index(id, bytes_written, self.write_total_bytes);
                }
                Ok(WriteEvent::Completed { id }) => {
                    self.mark_completed_by_index(id);
                    self.push_log(format!("[{}] write complete", id));
                }
                Ok(WriteEvent::Errored { id, message }) => {
                    self.mark_failed_by_index(id, message.clone());
                    self.push_log(format!("[!] [{}] error: {}", id, message));
                }
                Ok(WriteEvent::Finished { result }) => {
                    match result {
                        Ok(()) => self.push_log("=== Write complete ==="),
                        Err(e) => self.push_log(format!("[!] Write finished with error: {}", e)),
                    }
                    self.return_devices_online();
                    self.status = AppStatus::Finished;
                    keep = false;
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.push_log("[!] Write channel disconnected");
                    self.return_devices_online();
                    self.status = AppStatus::Finished;
                    keep = false;
                    break;
                }
            }
        }

        if keep {
            self.write_rx = Some(rx);
        }
    }

    pub fn return_devices_online(&mut self) {
        let numbers: Vec<u32> = self.devices.keys().copied().collect();
        for n in numbers {
            match crate::engine::set_disk_offline(n, false) {
                Ok(()) => self.push_log(format!("[#{}] back online", n)),
                Err(e) => self.push_log(format!("[#{}] failed to bring online: {}", n, e)),
            }
        }
    }

    pub fn auto_transition_finished(&mut self) {
        if self.status != AppStatus::Running || self.devices.is_empty() {
            return;
        }
        let all_done = self
            .devices
            .values()
            .all(|s| matches!(s.status, SlotStatus::Done | SlotStatus::Failed));
        if all_done {
            self.return_devices_online();
            self.status = AppStatus::Finished;
        }
    }

    // --- Действия пользователя ---

    pub fn start_write_flow(&mut self) {
        if self.image_path.is_none() {
            self.push_log("[!] Select an image first (I)");
            return;
        }
        if self.image_sha256.is_none() {
            self.push_log("[!] SHA-256 not yet computed");
            return;
        }

        let device_numbers = self.ready_device_numbers();
        if device_numbers.is_empty() {
            self.push_log("[!] No devices in Ready state");
            return;
        }
        if device_numbers.len() < self.target_count {
            self.push_log(format!(
                "[!] Need {} devices, ready {}",
                self.target_count,
                device_numbers.len()
            ));
            return;
        }

        if self.status == AppStatus::Running || self.prepare_rx.is_some() {
            self.push_log("[!] Operation already in progress");
            return;
        }

        let image_path = self.image_path.as_ref().unwrap().display().to_string();
        let total = self.image_size;

        self.status = AppStatus::Running;
        self.write_total_bytes = total;
        self.write_started_at = None;
        self.push_log(format!("Preparing {} devices…", device_numbers.len()));

        let (tx, rx) = channel();
        self.prepare_rx = Some(rx);

        thread::spawn(move || prepare_and_start(tx, device_numbers, image_path, total));
    }

    pub fn save_current_image_to_catalog(&mut self, size_bytes: u64, sha256: String) {
        let (Some(work_name), Some(path)) =
            (self.image_work_name.clone(), self.image_path.clone())
        else {
            return;
        };

        self.catalog.upsert(ImageEntry {
            work_name,
            path,
            size_bytes,
            sha256,
            added_at_unix: unix_now(),
        });

        match self.catalog.save(&self.catalog_path) {
            Ok(()) => self.push_log("Image added to catalog"),
            Err(e) => self.push_log(format!("[!] Catalog save failed: {}", e)),
        }
    }

    pub fn open_add_file_dialog(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Select image")
            .add_filter("Images", &["img", "iso", "raw"])
            .add_filter("All files", &["*"])
            .pick_file()
        else {
            return;
        };

        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let default_name = self
            .find_catalog_entry_by_path(&path)
            .map(|e| e.work_name.clone())
            .unwrap_or_else(|| {
                path.file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default()
            });

        self.pending_image_path = Some(path.clone());
        self.pending_image_size = size;
        self.input_buffer = default_name;
        self.screen = Screen::AddImageName;
        self.push_log(format!(
            "Adding image: {} ({:.2} GB)",
            path.display(),
            size as f64 / 1e9
        ));
    }

    pub fn open_ephemeral_file_dialog(&mut self) {
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

        self.image_path = Some(path.clone());
        self.image_size = size;
        self.image_work_name = Some(name.clone());
        self.image_sha256 = None;
        self.image_hash_progress = None;

        self.push_log(format!(
            "Opened once: {} ({:.2} GB)",
            path.display(),
            size as f64 / 1e9
        ));
        self.hash_job = Some(HashJob {
            rx: compute_sha256_async(path.display().to_string()),
            save_to_catalog: false,
        });

        self.screen = Screen::Main;
    }

    pub fn open_import_folder_dialog(&mut self) {
        if self.import_job.is_some() {
            self.push_log("[!] Import already in progress");
            return;
        }
        let Some(folder) = rfd::FileDialog::new()
            .set_title("Select folder with images")
            .pick_folder()
        else {
            return;
        };
        self.push_log(format!("[import] folder: {}", folder.display()));
        self.import_job = Some(spawn_folder_import(folder));
    }

    pub fn delete_selected_from_catalog(&mut self) {
        let idx = self.image_select_index;
        let Some(entry) = self.catalog.images.get(idx) else {
            return;
        };
        let name = entry.work_name.clone();

        if self.catalog.remove_by_name(&name) {
            match self.catalog.save(&self.catalog_path) {
                Ok(()) => self.push_log(format!("Removed: {}", name)),
                Err(e) => self.push_log(format!("[!] Catalog: {}", e)),
            }
            if self.image_select_index >= self.catalog.images.len() && self.image_select_index > 0 {
                self.image_select_index -= 1;
            }
        }
    }

    pub fn start_rename_selected(&mut self) {
        let idx = self.image_select_index;
        let Some(entry) = self.catalog.images.get(idx) else {
            return;
        };
        self.input_buffer = entry.work_name.clone();
        self.rename_target_index = Some(idx);
        self.screen = Screen::RenameImage;
    }

    pub fn commit_rename(&mut self) {
        let new_name = self.input_buffer.trim().to_string();
        if new_name.is_empty() {
            return;
        }
        let Some(idx) = self.rename_target_index.take() else {
            return;
        };
        let Some(old_name) = self.catalog.images.get(idx).map(|e| e.work_name.clone()) else {
            self.screen = Screen::ImageSelect;
            return;
        };

        match self.rename_in_catalog(&old_name, &new_name) {
            Ok(()) => {
                match self.catalog.save(&self.catalog_path) {
                    Ok(()) => self.push_log(format!("Renamed '{}' → '{}'", old_name, new_name)),
                    Err(e) => self.push_log(format!("[!] Catalog: {}", e)),
                }
                self.screen = Screen::ImageSelect;
            }
            Err(e) => {
                self.push_log(format!("[!] {}", e));
                self.rename_target_index = Some(idx);
            }
        }
        self.input_buffer.clear();
    }

    pub fn commit_new_image(&mut self) {
        let work_name = self.input_buffer.trim().to_string();
        if work_name.is_empty() {
            return;
        }
        let Some(path) = self.pending_image_path.take() else {
            return;
        };

        self.image_path = Some(path.clone());
        self.image_size = self.pending_image_size;
        self.image_work_name = Some(work_name.clone());
        self.image_sha256 = None;
        self.image_hash_progress = None;

        self.push_log(format!("Computing SHA-256 for '{}'…", work_name));
        self.hash_job = Some(HashJob {
            rx: compute_sha256_async(path.display().to_string()),
            save_to_catalog: true,
        });

        self.screen = Screen::Main;
        self.input_buffer.clear();
    }

    pub fn load_image_from_cli(&mut self, path: PathBuf) {
        if !path.exists() {
            self.push_log(format!("[!] File not found: {}", path.display()));
            return;
        }
        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        self.image_path = Some(path.clone());
        self.image_size = size;

        if let Some(entry) = self.find_catalog_entry_by_path(&path).cloned() {
            self.image_work_name = Some(entry.work_name.clone());
            self.image_sha256 = Some(entry.sha256);
            self.push_log(format!(
                "Found in catalog: {} (SHA-256 ready)",
                entry.work_name
            ));
            return;
        }

        self.push_log(format!(
            "Computing SHA-256: {} ({:.2} GB)…",
            path.display(),
            size as f64 / 1e9
        ));
        self.hash_job = Some(HashJob {
            rx: compute_sha256_async(path.display().to_string()),
            save_to_catalog: false,
        });
    }
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
        match crate::engine::unmount_all_volumes_on_disk(n) {
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
        if let Err(e) = crate::engine::set_disk_offline(n, true) {
            let _ = tx.send(PrepareResult::Log(format!(
                "[!] #{} offline failed: {}",
                n, e
            )));
        }

        match crate::engine::open_drive_write(n) {
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

fn timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let d = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    format!("[{}s]", d.as_secs())
}
