use std::fs::File;
use std::io::{self, Read};
use std::path::Path;
use std::sync::mpsc::{channel, Receiver};
use std::thread;
use std::time::Instant;

use sha2::{Digest, Sha256};
pub struct ImageInfo {
    pub path: String,
    pub size_bytes: u64,
    pub sha256: String,
}

/// Запускает фоновое вычисление SHA-256.
/// В TUI основной поток не блокируется — просто периодически
/// проверяет `try_recv()`.
#[derive(Debug)]
pub enum HashProgress {
    Progress { bytes_done: u64, bytes_total: u64 },
    Done { size_bytes: u64, sha256: String },
    Error(String),
}

/// Запускает фоновое вычисление SHA-256 с прогрессом.
pub fn compute_sha256_async(path: String) -> Receiver<HashProgress> {
    let (tx, rx) = channel();
    thread::spawn(move || {
        if let Err(e) = compute_with_progress(&path, &tx) {
            let _ = tx.send(HashProgress::Error(e.to_string()));
        }
    });
    rx
}

fn compute_with_progress(path: &str, tx: &std::sync::mpsc::Sender<HashProgress>) -> io::Result<()> {
    let p = Path::new(path);
    let mut file = File::open(p)?;
    let size_bytes = file.metadata()?.len();

    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 4 * 1024 * 1024]; // 4 MB блок
    let mut done: u64 = 0;
    let mut last_report = Instant::now();
    let mut last_report_bytes: u64 = 0;

    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        done += n as u64;

        // Отчёт не чаще 1 раза в 200 мс, чтобы не забивать канал
        if last_report.elapsed().as_millis() >= 200 {
            let _ = tx.send(HashProgress::Progress {
                bytes_done: done,
                bytes_total: size_bytes,
            });
            last_report = Instant::now();
            last_report_bytes = done;
        }
    }
    let _ = last_report_bytes; // чтобы не было warning

    let digest = hasher.finalize();
    let sha256 = hex::encode(digest);

    let _ = tx.send(HashProgress::Done {
        size_bytes,
        sha256,
    });
    Ok(())
}