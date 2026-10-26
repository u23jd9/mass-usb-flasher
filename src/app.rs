use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::time::Instant;

use crate::catalog::{Catalog, ImportProgress};
use crate::device::watcher::DeviceEvent;
use crate::device::DiskInfo;
use crate::engine::writer::WriteEvent;
use crate::i18n::{Lang, T};
use crate::image::HashProgress;
use crate::PrepareResult;

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

    fn recompute_status(&mut self) {
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

    /// Количество устройств, которые «в игре» — Ready до старта,
    /// либо Writing/Verifying/Done/Failed после старта.
    /// Используется в заголовке, чтобы не показывать «0/4» во время записи.
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

    // --- Метрики ---

    pub fn elapsed_secs(&self) -> Option<f64> {
        self.write_started_at.map(|t| t.elapsed().as_secs_f64())
    }

    /// Активные устройства — те, что сейчас пишутся или верифицируются.
    fn active_slots(&self) -> Vec<&DeviceSlot> {
        self.devices
            .values()
            .filter(|s| matches!(s.status, SlotStatus::Writing | SlotStatus::Verifying))
            .collect()
    }

    /// Средний прогресс по активным устройствам (байт на одно устройство).
    /// Используется для строки `Written: X / Y GB`.
    pub fn average_bytes_written(&self) -> u64 {
        let active = self.active_slots();
        if active.is_empty() {
            return 0;
        }
        let sum: u64 = active.iter().map(|s| s.bytes_written).sum();
        sum / active.len() as u64
    }

    /// Прогресс самой отстающей активной флешки.
    /// Используется для прогресс-бара в taskbar и в заголовке окна:
    /// когда дойдёт до конца самая медленная — все закончили.
    pub fn slowest_bytes_written(&self) -> u64 {
        self.active_slots()
            .iter()
            .map(|s| s.bytes_written)
            .min()
            .unwrap_or(0)
    }

    /// Средняя скорость на одно активное устройство, МБ/с.
    /// Суммарная скорость по всем устройствам смысла не имеет —
    /// это N независимых потоков.
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
}

fn timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let d = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    format!("[{}s]", d.as_secs())
}