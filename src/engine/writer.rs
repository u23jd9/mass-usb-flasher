use std::fs::File;
use std::io;
use std::sync::mpsc::{channel, Receiver};
use std::thread;

use bus_writer::{BusWriter, BusWriterMessage};

/// Событие прогресса, отправляемое в UI.
#[derive(Debug, Clone)]
pub enum WriteEvent {
    /// Устройство с индексом `id` записало `bytes` байт.
    Progress { id: usize, bytes_written: u64 },
    /// Устройство `id` успешно завершило запись.
    Completed { id: usize },
    /// Устройство `id` завершилось с ошибкой.
    Errored { id: usize, message: String },
    /// Вся операция завершена (успешно или с ошибками).
    Finished { result: Result<(), String> },
}

/// Размер буфера чтения образа.
/// 8 МБ — компромисс: меньше системных вызовов, чем при 4 МБ,
/// и при этом буфер не давит на память при 5+ устройствах.
const READ_BUFFER_BYTES: usize = 8 * 1024 * 1024;

/// Порог троттлинга прогресса — не чаще чем каждые 512 КБ на устройство.
const PROGRESS_STEP_BYTES: u64 = 512 * 1024;

/// Запускает параллельную запись образа `image_path` на список
/// уже открытых устройств `destinations`.
///
/// Каждый `File` должен быть открыт с флагами
/// `FILE_FLAG_NO_BUFFERING`, а тома на соответствующих дисках —
/// размонтированы.
pub fn start_write(
    image_path: String,
    mut destinations: Vec<File>,
    total_bytes: u64,
) -> Receiver<WriteEvent> {
    let (tx, rx) = channel();

    thread::spawn(move || {
        let image = match File::open(&image_path) {
            Ok(f) => f,
            Err(e) => {
                let _ = tx.send(WriteEvent::Finished {
                    result: Err(format!("Не удалось открыть образ: {}", e)),
                });
                return;
            }
        };

        // Увеличенный буфер чтения образа.
        let reader = io::BufReader::with_capacity(READ_BUFFER_BYTES, image);

        let tx_cb = tx.clone();
        let mut last_sent: Vec<u64> = vec![0; destinations.len()];

        let callback = move |msg: BusWriterMessage| match msg {
            BusWriterMessage::Written { id, bytes_written } => {
                // Троттлинг: не спамим UI чаще, чем каждые 512 КБ на устройство.
                if id < last_sent.len()
                    && bytes_written.saturating_sub(last_sent[id]) >= PROGRESS_STEP_BYTES
                {
                    last_sent[id] = bytes_written;
                    let _ = tx_cb.send(WriteEvent::Progress { id, bytes_written });
                }
            }
            BusWriterMessage::Completed { id } => {
                let _ = tx_cb.send(WriteEvent::Completed { id });
            }
            BusWriterMessage::Errored { id, why } => {
                let _ = tx_cb.send(WriteEvent::Errored {
                    id,
                    message: why.to_string(),
                });
            }
        };

        // Kill-switch: пока всегда false (не отменяем).
        let kill = || false;

        let result = BusWriter::new(reader, &mut destinations[..], callback, kill).write();

        let _ = tx.send(WriteEvent::Finished {
            result: result.map_err(|e| e.to_string()),
        });

        // total_bytes используется в UI, здесь просто гасим предупреждение.
        let _ = total_bytes;
    });

    rx
}

/// Событие одиночной перезаписи устройства (изолированная попытка повтора/восстановления).
#[derive(Debug, Clone)]
pub enum SingleWriteEvent {
    Progress { disk_number: u32, bytes_written: u64 },
    Status { disk_number: u32, message: String },
    Completed { disk_number: u32 },
    Failed { disk_number: u32, message: String },
    Log(String),
}

/// Запускает изолированную запись образа на один конкретный диск (для retry/auto-recovery).
/// Никак не затрагивает другие устройства.
pub fn start_single_drive_write(
    disk_number: u32,
    image_path: String,
    total_bytes: u64,
    attempt: u32,
    max_retries: u32,
) -> Receiver<SingleWriteEvent> {
    let (tx, rx) = channel();

    thread::spawn(move || {
        single_drive_worker(tx, disk_number, image_path, total_bytes, attempt, max_retries);
    });

    rx
}

fn single_drive_worker(
    tx: std::sync::mpsc::Sender<SingleWriteEvent>,
    disk_number: u32,
    image_path: String,
    total_bytes: u64,
    attempt: u32,
    max_retries: u32,
) {
    let _ = tx.send(SingleWriteEvent::Log(format!(
        "[#{}] Device auto-recovery: settling 1.5s before retry ({}/{})…",
        disk_number, attempt, max_retries
    )));
    thread::sleep(std::time::Duration::from_millis(1500));

    // Попытки переподключить и открыть устройство (до 5 попыток с интервалом 1 сек)
    let mut file = None;
    for i in 1..=5 {
        let _ = tx.send(SingleWriteEvent::Status {
            disk_number,
            message: format!("Reconnecting ({}/5)…", i),
        });

        let _ = crate::engine::unmount_all_volumes_on_disk(disk_number);
        let _ = crate::engine::set_disk_offline(disk_number, true);

        match crate::engine::open_drive_write(disk_number) {
            Ok(f) => {
                let _ = tx.send(SingleWriteEvent::Log(format!(
                    "[#{}] Reconnected & opened for rewrite on attempt {}",
                    disk_number, i
                )));
                file = Some(f);
                break;
            }
            Err(_) => {
                if i < 5 {
                    thread::sleep(std::time::Duration::from_millis(1000));
                }
            }
        }
    }

    let mut file = match file {
        Some(f) => f,
        None => {
            let _ = tx.send(SingleWriteEvent::Failed {
                disk_number,
                message: "Device reconnect timed out".to_string(),
            });
            return;
        }
    };

    let image_file = match File::open(&image_path) {
        Ok(f) => f,
        Err(e) => {
            let _ = tx.send(SingleWriteEvent::Failed {
                disk_number,
                message: format!("Cannot open image: {}", e),
            });
            return;
        }
    };

    let _ = tx.send(SingleWriteEvent::Status {
        disk_number,
        message: format!("Rewriting ({}/{})…", attempt, max_retries),
    });
    let _ = tx.send(SingleWriteEvent::Log(format!(
        "[#{}] Starting rewrite ({}/{})…",
        disk_number, attempt, max_retries
    )));

    const BUF_SIZE: usize = 4 * 1024 * 1024;
    let mut reader = io::BufReader::with_capacity(BUF_SIZE, image_file);
    let mut buffer = vec![0u8; BUF_SIZE];
    let mut written: u64 = 0;
    let mut last_reported: u64 = 0;

    use io::{Read, Write};
    loop {
        let bytes_read = match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) => {
                let _ = tx.send(SingleWriteEvent::Failed {
                    disk_number,
                    message: format!("Image read error: {}", e),
                });
                return;
            }
        };

        if let Err(e) = file.write_all(&buffer[..bytes_read]) {
            let _ = tx.send(SingleWriteEvent::Failed {
                disk_number,
                message: format!("Write error: {}", e),
            });
            return;
        }

        written += bytes_read as u64;
        if written.saturating_sub(last_reported) >= PROGRESS_STEP_BYTES
            || (total_bytes > 0 && written >= total_bytes)
        {
            last_reported = written;
            let _ = tx.send(SingleWriteEvent::Progress {
                disk_number,
                bytes_written: written,
            });
        }
    }

    let _ = crate::engine::flush_file(&file);
    crate::engine::update_disk_properties(&file);
    drop(file);
    let _ = crate::engine::set_disk_offline(disk_number, false);

    let _ = tx.send(SingleWriteEvent::Completed { disk_number });
    let _ = tx.send(SingleWriteEvent::Log(format!(
        "[#{}] Rewrite completed successfully! (attempt {}/{})",
        disk_number, attempt, max_retries
    )));
}