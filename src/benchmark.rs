use std::io::{self, Read, Seek, SeekFrom, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BenchmarkTestSize {
    Size256MB,
    Size512MB,
    Size1GB,
    Size2GB,
    Size4GB,
    FullDisk,
}

impl BenchmarkTestSize {
    pub fn bytes(&self, disk_size_bytes: u64) -> u64 {
        let max_safe = disk_size_bytes.saturating_sub(8 * 1024 * 1024);
        let requested = match self {
            BenchmarkTestSize::Size256MB => 256 * 1024 * 1024,
            BenchmarkTestSize::Size512MB => 512 * 1024 * 1024,
            BenchmarkTestSize::Size1GB => 1024 * 1024 * 1024,
            BenchmarkTestSize::Size2GB => 2 * 1024 * 1024 * 1024,
            BenchmarkTestSize::Size4GB => 4 * 1024 * 1024 * 1024,
            BenchmarkTestSize::FullDisk => max_safe,
        };
        requested.min(max_safe).max(64 * 1024 * 1024)
    }

    pub fn label_en(&self) -> &'static str {
        match self {
            BenchmarkTestSize::Size256MB => "256 MB",
            BenchmarkTestSize::Size512MB => "512 MB",
            BenchmarkTestSize::Size1GB => "1 GB (Recommended)",
            BenchmarkTestSize::Size2GB => "2 GB",
            BenchmarkTestSize::Size4GB => "4 GB",
            BenchmarkTestSize::FullDisk => "Full Disk",
        }
    }

    pub fn label_ru(&self) -> &'static str {
        match self {
            BenchmarkTestSize::Size256MB => "256 МБ",
            BenchmarkTestSize::Size512MB => "512 МБ",
            BenchmarkTestSize::Size1GB => "1 ГБ (Рекомендуется)",
            BenchmarkTestSize::Size2GB => "2 ГБ",
            BenchmarkTestSize::Size4GB => "4 ГБ",
            BenchmarkTestSize::FullDisk => "Весь объём флешки",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BenchmarkMode {
    Standard,
    Detailed,
}

impl Default for BenchmarkMode {
    fn default() -> Self {
        BenchmarkMode::Detailed
    }
}

impl BenchmarkMode {
    pub fn label_en(&self) -> &'static str {
        match self {
            BenchmarkMode::Standard => "Standard (Continuous)",
            BenchmarkMode::Detailed => "Detailed (Thermal Stress & Recovery)",
        }
    }

    pub fn label_ru(&self) -> &'static str {
        match self {
            BenchmarkMode::Standard => "Обычный (Сплошной)",
            BenchmarkMode::Detailed => "Детальный (Стресс и перегрев)",
        }
    }
}

#[derive(Debug, Clone)]
pub struct BenchmarkConfig {
    pub test_size: BenchmarkTestSize,
    pub mode: BenchmarkMode,
    pub wipe_partitions: bool,
}

impl Default for BenchmarkConfig {
    fn default() -> Self {
        Self {
            test_size: BenchmarkTestSize::Size1GB,
            mode: BenchmarkMode::Detailed,
            wipe_partitions: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum BenchmarkPhase {
    Idle,
    WipingPartitions,
    Writing,
    Flushing,
    Reading,
    Done,
    Failed(String),
    Cancelled,
}

#[derive(Debug, Clone, Default)]
pub struct StabilityMetrics {
    pub avg_mbps: f64,
    pub min_mbps: f64,
    pub max_mbps: f64,
    pub std_dev_mbps: f64,
    pub stability_pct: f64, // 0.0 - 100.0%
    pub drop_ratio_pct: f64, // min / avg * 100.0%
    pub controller_disconnects: u32,
    pub thermal_throttled: bool,
}

impl StabilityMetrics {
    pub fn compute(
        samples: &[f32],
        total_bytes: u64,
        total_duration: Duration,
        disconnects: u32,
    ) -> Self {
        if samples.is_empty() || total_duration.is_zero() {
            return Self {
                controller_disconnects: disconnects,
                ..Default::default()
            };
        }

        let total_secs = total_duration.as_secs_f64();
        let avg_mbps = if total_secs > 0.0 {
            (total_bytes as f64 / 1_048_576.0) / total_secs
        } else {
            0.0
        };

        let mut min_mbps: f64 = f64::MAX;
        let mut max_mbps: f64 = 0.0;
        let mut sum: f64 = 0.0;
        let mut count = 0;

        for &s in samples {
            let val = s as f64;
            if val > 0.05 {
                min_mbps = min_mbps.min(val);
                max_mbps = max_mbps.max(val);
                sum += val;
                count += 1;
            }
        }

        if count == 0 {
            min_mbps = 0.0;
        }

        let mean = if count > 0 { sum / count as f64 } else { avg_mbps };
        let mut variance_sum = 0.0;
        for &s in samples {
            let val = s as f64;
            if val > 0.05 {
                let diff = val - mean;
                variance_sum += diff * diff;
            }
        }

        let std_dev = if count > 1 {
            (variance_sum / count as f64).sqrt()
        } else {
            0.0
        };

        let cv = if mean > 0.0 { std_dev / mean } else { 0.0 };
        let stability_pct = ((1.0 - cv).max(0.0) * 100.0).clamp(0.0, 100.0);
        let drop_ratio_pct = if mean > 0.0 {
            (min_mbps / mean * 100.0).clamp(0.0, 100.0)
        } else {
            0.0
        };

        let thermal_throttled = min_mbps > 0.0
            && (min_mbps < avg_mbps * 0.35 || drop_ratio_pct < 35.0)
            && max_mbps > avg_mbps * 1.3;

        Self {
            avg_mbps,
            min_mbps: if min_mbps == f64::MAX { 0.0 } else { min_mbps },
            max_mbps,
            std_dev_mbps: std_dev,
            stability_pct,
            drop_ratio_pct,
            controller_disconnects: disconnects,
            thermal_throttled,
        }
    }

    pub fn verdict_en(&self) -> String {
        if self.controller_disconnects > 0 {
            format!("Overheat: controller dropped ({}x, recovered)", self.controller_disconnects)
        } else if self.thermal_throttled {
            format!("Thermal Throttle ({:.0}% stability, drops)", self.stability_pct)
        } else if self.stability_pct >= 85.0 {
            "Excellent (Highly Stable)".to_string()
        } else if self.stability_pct >= 70.0 {
            "Good (Minor Fluctuations)".to_string()
        } else if self.stability_pct >= 45.0 {
            "Fair (Noticeable Speed Drops)".to_string()
        } else {
            "Poor (Severe Throttling / Stalls)".to_string()
        }
    }

    pub fn verdict_ru(&self) -> String {
        if self.controller_disconnects > 0 {
            format!("Перегрев: контроллер сбоил ({}x, восстановлен)", self.controller_disconnects)
        } else if self.thermal_throttled {
            format!("Термотроттлинг ({:.0}% стабильность, просадки)", self.stability_pct)
        } else if self.stability_pct >= 85.0 {
            "Отличная (Высокая стабильность)".to_string()
        } else if self.stability_pct >= 70.0 {
            "Хорошая (Небольшие колебания)".to_string()
        } else if self.stability_pct >= 45.0 {
            "Умеренная (Заметные просадки)".to_string()
        } else {
            "Низкая (Сильный троттлинг / задержки)".to_string()
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct BenchmarkResult {
    pub disk_number: u32,
    pub test_bytes: u64,
    pub mode: BenchmarkMode,
    pub write_metrics: StabilityMetrics,
    pub write_samples: Vec<f32>,
    pub read_metrics: StabilityMetrics,
    pub read_samples: Vec<f32>,
    pub verify_errors: u64,
    pub controller_disconnects: u32,
    pub total_elapsed_secs: f64,
    pub partitions_wiped: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub enum BenchmarkEvent {
    Log(String),
    PhaseChanged {
        disk_number: u32,
        phase: BenchmarkPhase,
    },
    Progress {
        disk_number: u32,
        phase: BenchmarkPhase,
        overall_progress: f32, // 0.0 - 1.0
        current_speed_mbps: f64,
        bytes_done: u64,
        total_bytes: u64,
    },
    SampleAdded {
        disk_number: u32,
        is_write: bool,
        speed_mbps: f32,
    },
    Finished {
        disk_number: u32,
        result: BenchmarkResult,
    },
}

#[derive(Debug, Clone)]
pub struct BenchmarkDriveState {
    pub disk_number: u32,
    pub phase: BenchmarkPhase,
    pub progress: f32,
    pub current_speed_mbps: f64,
    pub bytes_done: u64,
    pub total_bytes: u64,
    pub live_write_samples: Vec<f32>,
    pub live_read_samples: Vec<f32>,
    pub result: Option<BenchmarkResult>,
    pub is_active: bool,
}

impl BenchmarkDriveState {
    pub fn new(disk_number: u32) -> Self {
        Self {
            disk_number,
            phase: BenchmarkPhase::Idle,
            progress: 0.0,
            current_speed_mbps: 0.0,
            bytes_done: 0,
            total_bytes: 0,
            live_write_samples: Vec::new(),
            live_read_samples: Vec::new(),
            result: None,
            is_active: false,
        }
    }
}

/// Затирает таблицу разделов флешки (MBR в начале и резервную копию GPT в конце диска).
pub fn wipe_partition_table(disk_number: u32, disk_size_bytes: u64) -> io::Result<()> {
    // 1. Принудительно размонтируем все тома
    let _ = crate::engine::unmount_all_volumes_on_disk(disk_number);

    // 2. На Windows переводим в Offline для снятия блокировок ФС
    let _ = crate::engine::set_disk_offline(disk_number, true);

    // 3. Открываем диск для прямой записи
    let mut file = crate::engine::open_drive_write(disk_number)?;

    // 4. Затираем первые 4 МБ (MBR, защитная MBR, GPT-заголовок, записи разделов GPT, boot-секторы)
    const WIPE_CHUNK: usize = 4 * 1024 * 1024;
    let zeros = vec![0u8; WIPE_CHUNK];

    file.seek(SeekFrom::Start(0))?;
    file.write_all(&zeros)?;

    // 5. Затираем резервную таблицу GPT в конце диска (Backup GPT header & partition table)
    if disk_size_bytes > (8 * 1024 * 1024) {
        let backup_offset = (disk_size_bytes.saturating_sub(WIPE_CHUNK as u64) / 4096) * 4096;
        if file.seek(SeekFrom::Start(backup_offset)).is_ok() {
            let _ = file.write_all(&zeros);
        }
    }

    // 6. Сбрасываем кэш и уведомляем ОС об очистке разделов
    let _ = crate::engine::flush_file(&file);
    crate::engine::update_disk_properties(&file);
    drop(file);

    #[cfg(not(windows))]
    {
        crate::engine::sync_all();
        // Даём udev/ядру успокоиться и завершить перечитку разделов, не блокируя устройство
        thread::sleep(Duration::from_millis(600));
    }

    Ok(())
}

/// Генерирует детерминированный псевдослучайный тестовый блок (SplitMix64).
/// Предотвращает аппаратное сжатие контроллерами и позволяет побитовую верификацию при чтении.
fn generate_test_block(block_idx: u64, buf: &mut [u8]) {
    let mut state = (block_idx.wrapping_mul(0x9E3779B97F4A7C15)).wrapping_add(0xD168AAAD);
    for chunk in buf.chunks_exact_mut(8) {
        state ^= state >> 30;
        state = state.wrapping_mul(0xBF58476D1CE4E5B9);
        state ^= state >> 27;
        state = state.wrapping_mul(0x94D049BB133111EB);
        state ^= state >> 31;
        chunk.copy_from_slice(&state.to_le_bytes());
    }
}

/// Запускает бенчмарк для списка дисков.
pub fn start_benchmark_sessions(
    drives: Vec<(u32, u64)>,
    config: BenchmarkConfig,
) -> (Receiver<BenchmarkEvent>, Arc<AtomicBool>) {
    let (tx, rx) = channel();
    let cancel_flag = Arc::new(AtomicBool::new(false));

    for (disk_number, disk_size_bytes) in drives {
        let tx_clone = tx.clone();
        let cancel_clone = cancel_flag.clone();
        let cfg_clone = config.clone();

        thread::spawn(move || {
            run_single_benchmark_worker(
                disk_number,
                disk_size_bytes,
                cfg_clone,
                cancel_clone,
                tx_clone,
            );
        });
    }

    (rx, cancel_flag)
}

fn run_single_benchmark_worker(
    disk_number: u32,
    disk_size_bytes: u64,
    config: BenchmarkConfig,
    cancel_flag: Arc<AtomicBool>,
    tx: Sender<BenchmarkEvent>,
) {
    let total_start = Instant::now();
    let test_bytes = config.test_size.bytes(disk_size_bytes);

    let mut result = BenchmarkResult {
        disk_number,
        test_bytes,
        mode: config.mode,
        partitions_wiped: false,
        ..Default::default()
    };

    if cancel_flag.load(Ordering::Relaxed) {
        let _ = tx.send(BenchmarkEvent::PhaseChanged {
            disk_number,
            phase: BenchmarkPhase::Cancelled,
        });
        return;
    }

    // ----------------------------------------------------
    // ШАГ 1: Затирание таблицы разделов (MBR + GPT)
    // ----------------------------------------------------
    if config.wipe_partitions {
        let _ = tx.send(BenchmarkEvent::PhaseChanged {
            disk_number,
            phase: BenchmarkPhase::WipingPartitions,
        });
        let _ = tx.send(BenchmarkEvent::Progress {
            disk_number,
            phase: BenchmarkPhase::WipingPartitions,
            overall_progress: 0.02,
            current_speed_mbps: 0.0,
            bytes_done: 0,
            total_bytes: test_bytes,
        });
        let _ = tx.send(BenchmarkEvent::Log(format!(
            "[#{}] [Benchmark] Step 1/3: Wiping partition table (erasing MBR and secondary GPT)...",
            disk_number
        )));

        match wipe_partition_table(disk_number, disk_size_bytes) {
            Ok(()) => {
                result.partitions_wiped = true;
                let _ = tx.send(BenchmarkEvent::Log(format!(
                    "[#{}] [Benchmark] Partition table cleanly wiped ✔",
                    disk_number
                )));
                // Дополнительная задержка после затирания для стабилизации шины и диска перед началом стресс-теста
                thread::sleep(Duration::from_millis(500));
            }
            Err(e) => {
                let err_msg = format!("Failed to wipe partition table: {}", e);
                let _ = tx.send(BenchmarkEvent::Log(format!(
                    "[!] [#{}] [Benchmark] {}",
                    disk_number, err_msg
                )));
                result.error = Some(err_msg.clone());
                let _ = tx.send(BenchmarkEvent::PhaseChanged {
                    disk_number,
                    phase: BenchmarkPhase::Failed(err_msg),
                });
                let _ = crate::engine::set_disk_offline(disk_number, false);
                return;
            }
        }
    }

    if cancel_flag.load(Ordering::Relaxed) {
        let _ = tx.send(BenchmarkEvent::PhaseChanged {
            disk_number,
            phase: BenchmarkPhase::Cancelled,
        });
        let _ = crate::engine::set_disk_offline(disk_number, false);
        return;
    }

    // ----------------------------------------------------
    // ШАГ 2: Бенчмарк последовательной записи
    // ----------------------------------------------------
    let _ = tx.send(BenchmarkEvent::PhaseChanged {
        disk_number,
        phase: BenchmarkPhase::Writing,
    });
    let _ = tx.send(BenchmarkEvent::Log(format!(
        "[#{}] [Benchmark] Step 2/3: Starting write benchmark ({:.2} GB)...",
        disk_number,
        test_bytes as f64 / 1e9
    )));

    const CHUNK_SIZE: usize = 1024 * 1024; // 1 МБ для более плавной записи без скачков нагрузки на шину
    let mut pattern_buf = vec![0u8; CHUNK_SIZE];

    let mut write_file = match crate::engine::open_drive_write(disk_number) {
        Ok(f) => f,
        Err(e) => {
            let err_msg = format!("Failed to open drive for writing: {}", e);
            result.error = Some(err_msg.clone());
            let _ = tx.send(BenchmarkEvent::PhaseChanged {
                disk_number,
                phase: BenchmarkPhase::Failed(err_msg),
            });
            let _ = crate::engine::set_disk_offline(disk_number, false);
            return;
        }
    };

    let write_start = Instant::now();
    let mut written: u64 = 0;
    let mut block_idx: u64 = 0;
    let mut last_sample_time = Instant::now();
    let mut bytes_since_sample: u64 = 0;
    let mut controller_disconnects: u32 = 0;

    while written < test_bytes {
        if cancel_flag.load(Ordering::Relaxed) {
            let _ = tx.send(BenchmarkEvent::PhaseChanged {
                disk_number,
                phase: BenchmarkPhase::Cancelled,
            });
            let _ = crate::engine::set_disk_offline(disk_number, false);
            return;
        }

        let to_write = (test_bytes - written).min(CHUNK_SIZE as u64) as usize;
        generate_test_block(block_idx, &mut pattern_buf[..to_write]);

        let write_res = write_file.write_all(&pattern_buf[..to_write]);
        if let Err(e) = write_res {
            // Если произошла ошибка записи: даем короткую паузу и пробуем повторить операцию,
            // чтобы исключить мелкие сбои из-за кратковременной перегрузки буфера или шины
            let mut recovered = false;
            let max_retries = if config.mode == BenchmarkMode::Detailed { 5 } else { 2 };

            for retry in 1..=max_retries {
                if cancel_flag.load(Ordering::Relaxed) {
                    let _ = crate::engine::set_disk_offline(disk_number, false);
                    return;
                }
                controller_disconnects += 1;
                let cool_time = if config.mode == BenchmarkMode::Detailed { 3000 } else { 800 };
                let _ = tx.send(BenchmarkEvent::Log(format!(
                    "[#{}] [Thermal/Bus Diagnostic] Write glitch/drop at {} MB: {}. Pausing {}ms to recover… (retry {}/{})",
                    disk_number, written / 1_048_576, e, cool_time, retry, max_retries
                )));

                // Добавляем 0 МБ/с в сэмплы для наглядного отображения термического/шинного провала на графике
                result.write_samples.push(0.0);
                let _ = tx.send(BenchmarkEvent::SampleAdded {
                    disk_number,
                    is_write: true,
                    speed_mbps: 0.0,
                });

                thread::sleep(Duration::from_millis(cool_time));

                let _ = crate::engine::unmount_all_volumes_on_disk(disk_number);
                let _ = crate::engine::set_disk_offline(disk_number, true);

                if let Ok(mut new_file) = crate::engine::open_drive_write(disk_number) {
                    if new_file.seek(SeekFrom::Start(written)).is_ok() {
                        if new_file.write_all(&pattern_buf[..to_write]).is_ok() {
                            write_file = new_file;
                            recovered = true;
                            let _ = tx.send(BenchmarkEvent::Log(format!(
                                "[#{}] [Recovery] Successfully resumed write at {} MB ✔",
                                disk_number, written / 1_048_576
                            )));
                            last_sample_time = Instant::now();
                            bytes_since_sample = 0;
                            break;
                        }
                    }
                }
            }

            if !recovered {
                let err_msg = if controller_disconnects > 0 {
                    format!("Drive I/O failure/disconnect at {} MB: {}", written / 1_048_576, e)
                } else {
                    format!("Write error at {} MB: {}", written / 1_048_576, e)
                };
                result.error = Some(err_msg.clone());
                result.controller_disconnects = controller_disconnects;
                let _ = tx.send(BenchmarkEvent::PhaseChanged {
                    disk_number,
                    phase: BenchmarkPhase::Failed(err_msg),
                });
                let _ = crate::engine::set_disk_offline(disk_number, false);
                return;
            }
        }

        written += to_write as u64;
        bytes_since_sample += to_write as u64;
        block_idx += 1;

        let elapsed = last_sample_time.elapsed();
        if elapsed >= Duration::from_millis(250) || written >= test_bytes {
            let speed_mbps = (bytes_since_sample as f64 / 1_048_576.0) / elapsed.as_secs_f64();
            result.write_samples.push(speed_mbps as f32);

            let overall_prog = 0.05 + 0.45 * (written as f32 / test_bytes as f32);
            let _ = tx.send(BenchmarkEvent::SampleAdded {
                disk_number,
                is_write: true,
                speed_mbps: speed_mbps as f32,
            });
            let _ = tx.send(BenchmarkEvent::Progress {
                disk_number,
                phase: BenchmarkPhase::Writing,
                overall_progress: overall_prog,
                current_speed_mbps: speed_mbps,
                bytes_done: written,
                total_bytes: test_bytes,
            });

            last_sample_time = Instant::now();
            bytes_since_sample = 0;
        }
    }

    // Сброс кэша контроллера на физический NAND
    let _ = tx.send(BenchmarkEvent::PhaseChanged {
        disk_number,
        phase: BenchmarkPhase::Flushing,
    });
    let _ = crate::engine::flush_file(&write_file);
    drop(write_file);

    #[cfg(not(windows))]
    crate::engine::sync_all();

    let write_duration = write_start.elapsed();
    result.write_metrics = StabilityMetrics::compute(&result.write_samples, written, write_duration, controller_disconnects);

    let _ = tx.send(BenchmarkEvent::Log(format!(
        "[#{}] [Benchmark] Write complete: avg {:.1} MB/s (min {:.1}, max {:.1}, stability {:.0}%, overheat disconnects: {})",
        disk_number,
        result.write_metrics.avg_mbps,
        result.write_metrics.min_mbps,
        result.write_metrics.max_mbps,
        result.write_metrics.stability_pct,
        controller_disconnects
    )));

    if cancel_flag.load(Ordering::Relaxed) {
        let _ = tx.send(BenchmarkEvent::PhaseChanged {
            disk_number,
            phase: BenchmarkPhase::Cancelled,
        });
        let _ = crate::engine::set_disk_offline(disk_number, false);
        return;
    }

    // ----------------------------------------------------
    // ШАГ 3: Бенчмарк последовательного чтения и верификация
    // ----------------------------------------------------
    let _ = tx.send(BenchmarkEvent::PhaseChanged {
        disk_number,
        phase: BenchmarkPhase::Reading,
    });
    let _ = tx.send(BenchmarkEvent::Log(format!(
        "[#{}] [Benchmark] Step 3/3: Starting read & data stability verification ({:.2} GB)...",
        disk_number,
        test_bytes as f64 / 1e9
    )));

    let mut read_file = match crate::device::open_drive_read(disk_number) {
        Ok(f) => f,
        Err(e) => {
            let err_msg = format!("Failed to open drive for reading: {}", e);
            result.error = Some(err_msg.clone());
            let _ = tx.send(BenchmarkEvent::PhaseChanged {
                disk_number,
                phase: BenchmarkPhase::Failed(err_msg),
            });
            let _ = crate::engine::set_disk_offline(disk_number, false);
            return;
        }
    };

    let read_start = Instant::now();
    let mut read_bytes: u64 = 0;
    let mut read_block_idx: u64 = 0;
    let mut read_buf = vec![0u8; CHUNK_SIZE];
    let mut expected_buf = vec![0u8; CHUNK_SIZE];
    let mut last_read_sample = Instant::now();
    let mut bytes_since_read_sample: u64 = 0;
    let mut verify_errors: u64 = 0;

    while read_bytes < test_bytes {
        if cancel_flag.load(Ordering::Relaxed) {
            let _ = tx.send(BenchmarkEvent::PhaseChanged {
                disk_number,
                phase: BenchmarkPhase::Cancelled,
            });
            let _ = crate::engine::set_disk_offline(disk_number, false);
            return;
        }

        let to_read = (test_bytes - read_bytes).min(CHUNK_SIZE as u64) as usize;
        let read_res = read_file.read_exact(&mut read_buf[..to_read]);
        if let Err(e) = read_res {
            if config.mode == BenchmarkMode::Detailed && controller_disconnects < 5 {
                controller_disconnects += 1;
                let _ = tx.send(BenchmarkEvent::Log(format!(
                    "[#{}] [Thermal Diagnostic] Read error at {} MB: {}. Pausing 3s for controller cool-down…",
                    disk_number, read_bytes / 1_048_576, e
                )));

                result.read_samples.push(0.0);
                let _ = tx.send(BenchmarkEvent::SampleAdded {
                    disk_number,
                    is_write: false,
                    speed_mbps: 0.0,
                });

                thread::sleep(Duration::from_millis(3000));

                let mut recovered = false;
                for _ in 1..=5 {
                    if cancel_flag.load(Ordering::Relaxed) {
                        let _ = crate::engine::set_disk_offline(disk_number, false);
                        return;
                    }
                    if let Ok(mut new_file) = crate::device::open_drive_read(disk_number) {
                        if new_file.seek(SeekFrom::Start(read_bytes)).is_ok() {
                            read_file = new_file;
                            recovered = true;
                            let _ = tx.send(BenchmarkEvent::Log(format!(
                                "[#{}] [Thermal Diagnostic] Reconnected! Resuming read test from {} MB",
                                disk_number, read_bytes / 1_048_576
                            )));
                            last_read_sample = Instant::now();
                            bytes_since_read_sample = 0;
                            break;
                        }
                    }
                    thread::sleep(Duration::from_millis(1000));
                }

                if recovered {
                    continue;
                }
            }

            let err_msg = format!("Read error at {} MB: {}", read_bytes / 1_048_576, e);
            result.error = Some(err_msg.clone());
            result.controller_disconnects = controller_disconnects;
            let _ = tx.send(BenchmarkEvent::PhaseChanged {
                disk_number,
                phase: BenchmarkPhase::Failed(err_msg),
            });
            let _ = crate::engine::set_disk_offline(disk_number, false);
            return;
        }

        // Побитовая верификация с эталонным сгенерированным блоком
        generate_test_block(read_block_idx, &mut expected_buf[..to_read]);
        if read_buf[..to_read] != expected_buf[..to_read] {
            verify_errors += 1;
        }

        read_bytes += to_read as u64;
        bytes_since_read_sample += to_read as u64;
        read_block_idx += 1;

        let elapsed = last_read_sample.elapsed();
        if elapsed >= Duration::from_millis(250) || read_bytes >= test_bytes {
            let speed_mbps =
                (bytes_since_read_sample as f64 / 1_048_576.0) / elapsed.as_secs_f64();
            result.read_samples.push(speed_mbps as f32);

            let overall_prog = 0.55 + 0.45 * (read_bytes as f32 / test_bytes as f32);
            let _ = tx.send(BenchmarkEvent::SampleAdded {
                disk_number,
                is_write: false,
                speed_mbps: speed_mbps as f32,
            });
            let _ = tx.send(BenchmarkEvent::Progress {
                disk_number,
                phase: BenchmarkPhase::Reading,
                overall_progress: overall_prog,
                current_speed_mbps: speed_mbps,
                bytes_done: read_bytes,
                total_bytes: test_bytes,
            });

            last_read_sample = Instant::now();
            bytes_since_read_sample = 0;
        }
    }

    drop(read_file);

    // Возвращаем диск в Online на Windows
    let _ = crate::engine::set_disk_offline(disk_number, false);

    let read_duration = read_start.elapsed();
    result.read_metrics =
        StabilityMetrics::compute(&result.read_samples, read_bytes, read_duration, 0);
    result.controller_disconnects = controller_disconnects;
    result.verify_errors = verify_errors;
    result.total_elapsed_secs = total_start.elapsed().as_secs_f64();

    let _ = tx.send(BenchmarkEvent::Log(format!(
        "[#{}] [Benchmark] Read complete: avg {:.1} MB/s (min {:.1}, max {:.1}, stability {:.0}%), Errors: {}",
        disk_number,
        result.read_metrics.avg_mbps,
        result.read_metrics.min_mbps,
        result.read_metrics.max_mbps,
        result.read_metrics.stability_pct,
        result.verify_errors
    )));

    let _ = tx.send(BenchmarkEvent::PhaseChanged {
        disk_number,
        phase: BenchmarkPhase::Done,
    });
    let _ = tx.send(BenchmarkEvent::Finished {
        disk_number,
        result,
    });
}
