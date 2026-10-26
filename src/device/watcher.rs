use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;
use std::time::Duration;

use super::DiskInfo;

#[derive(Debug, Clone)]
pub enum DeviceEvent {
    Arrived(DiskInfo),
    Removed { number: u32, path: String },
}

#[derive(Debug, Clone, Default)]
pub struct DeviceFilter {
    pub usb_only: bool,
    pub min_size_bytes: u64,
    pub max_size_bytes: u64,
}

impl DeviceFilter {
    pub fn matches(&self, d: &DiskInfo) -> bool {
        if self.usb_only && !d.is_usb {
            return false;
        }
        if self.min_size_bytes > 0 && d.size_bytes < self.min_size_bytes {
            return false;
        }
        if self.max_size_bytes > 0 && d.size_bytes > self.max_size_bytes {
            return false;
        }
        true
    }
}

pub struct WatcherConfig {
    pub poll_interval: Duration,
    pub max_disks: u32,
    pub filter: DeviceFilter,
}

pub struct WatcherHandle {
    pub events: Receiver<DeviceEvent>,
}

/// Запускает watcher в отдельном потоке.
/// Поток живёт всё время работы процесса (остановка — при выходе).
pub fn spawn(config: WatcherConfig) -> WatcherHandle {
    let (tx, rx) = channel();
    thread::spawn(move || run_watcher(config, tx));
    WatcherHandle { events: rx }
}

fn run_watcher(config: WatcherConfig, tx: Sender<DeviceEvent>) {
    let mut known: HashMap<u32, DiskInfo> = HashMap::new();

    loop {
        let all = super::enumerate_disks(config.max_disks);

        let mut current: HashMap<u32, DiskInfo> = HashMap::new();
        for d in all {
            if config.filter.matches(&d) {
                current.insert(d.number, d);
            }
        }

        // Появились: есть в current, нет в known
        for (n, d) in &current {
            if !known.contains_key(n) {
                let _ = tx.send(DeviceEvent::Arrived(d.clone()));
            }
        }

        // Пропали: есть в known, нет в current (включая случаи,
        // когда диск отключился, стал недоступен или вышел из фильтра)
        for (n, d) in &known {
            if !current.contains_key(n) {
                let _ = tx.send(DeviceEvent::Removed {
                    number: *n,
                    path: d.path.clone(),
                });
            }
        }

        known = current;
        thread::sleep(config.poll_interval);
    }
}