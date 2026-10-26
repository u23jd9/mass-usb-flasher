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