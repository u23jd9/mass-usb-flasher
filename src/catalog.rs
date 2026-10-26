use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const MANIFEST_NAME: &str = ".muf-images.json";
const MANIFEST_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageEntry {
    pub work_name: String,
    pub path: PathBuf,
    pub size_bytes: u64,
    pub sha256: String,
    pub added_at_unix: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ManifestEntry {
    /// Относительный путь от папки манифеста, разделитель `/`.
    relative_path: String,
    work_name: String,
    size_bytes: u64,
    sha256: String,
    added_at_unix: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Manifest {
    version: u32,
    #[serde(default)]
    images: Vec<ManifestEntry>,
}

impl Default for Manifest {
    fn default() -> Self {
        Manifest {
            version: MANIFEST_VERSION,
            images: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Catalog {
    #[serde(default)]
    pub images: Vec<ImageEntry>,
}

impl Catalog {
    pub fn load_or_default(path: &Path) -> Self {
        match fs::read_to_string(path) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => Catalog::default(),
        }
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let s = serde_json::to_string_pretty(self)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        fs::write(path, s)
    }

    pub fn upsert(&mut self, entry: ImageEntry) {
        self.images
            .retain(|e| e.work_name != entry.work_name && e.path != entry.path);
        self.images.push(entry);
        self.images.sort_by(|a, b| a.work_name.cmp(&b.work_name));
    }

    pub fn remove_by_name(&mut self, work_name: &str) -> bool {
        let before = self.images.len();
        self.images.retain(|e| e.work_name != work_name);
        self.images.len() != before
    }

    pub fn rename(&mut self, old_name: &str, new_name: &str) -> Result<(), String> {
        if old_name == new_name {
            return Ok(());
        }
        if self.images.iter().any(|e| e.work_name == new_name) {
            return Err(format!("имя '{}' уже занято", new_name));
        }
        let entry = self
            .images
            .iter_mut()
            .find(|e| e.work_name == old_name)
            .ok_or_else(|| "образ не найден".to_string())?;
        entry.work_name = new_name.to_string();
        self.images.sort_by(|a, b| a.work_name.cmp(&b.work_name));
        Ok(())
    }
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn default_catalog_path() -> PathBuf {
    if let Ok(home) = std::env::var("USERPROFILE") {
        PathBuf::from(home).join(".muf").join("catalog.json")
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".muf").join("catalog.json")
    } else {
        PathBuf::from("muf_catalog.json")
    }
}

// --- Импорт папки ---

#[derive(Debug, Clone)]
pub enum ImportProgress {
    Scanning { total: usize },
    HashingFile {
        current: usize,
        total: usize,
        name: String,
        bytes_done: u64,
        bytes_total: u64,
    },
    FileDone { name: String },
    Done { imported: usize, entries: Vec<ImageEntry> },
    Error(String),
}

/// Запускает фоновый импорт папки.
pub fn spawn_folder_import(folder: PathBuf) -> Receiver<ImportProgress> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        run_import(folder, tx);
    });
    rx
}

fn run_import(folder: PathBuf, tx: std::sync::mpsc::Sender<ImportProgress>) {
    let send = |p: ImportProgress| {
        let _ = tx.send(p);
    };

    if !folder.is_dir() {
        send(ImportProgress::Error(format!(
            "не директория: {}",
            folder.display()
        )));
        return;
    }

    let files = collect_images(&folder);
    send(ImportProgress::Scanning { total: files.len() });

    if files.is_empty() {
        send(ImportProgress::Done {
            imported: 0,
            entries: Vec::new(),
        });
        return;
    }

    let old = load_manifest(&folder);
    let mut by_rel: HashMap<String, ManifestEntry> = old
        .images
        .into_iter()
        .map(|e| (e.relative_path.clone(), e))
        .collect();

    let mut new_entries: Vec<ManifestEntry> = Vec::new();
    let mut entries: Vec<ImageEntry> = Vec::new();

    for (i, file) in files.iter().enumerate() {
        let Some(rel) = to_relative(&folder, file) else {
            continue;
        };
        let size = fs::metadata(file).map(|m| m.len()).unwrap_or(0);

        let (work_name, sha256) = match by_rel.remove(&rel) {
            Some(old) if old.size_bytes == size => (old.work_name, old.sha256),
            Some(old) => {
                let w = old.work_name;
                match hash_file(file, &tx, i + 1, files.len()) {
                    Ok(h) => (w, h),
                    Err(e) => {
                        send(ImportProgress::Error(e));
                        return;
                    }
                }
            }
            None => {
                let w = default_work_name(file);
                match hash_file(file, &tx, i + 1, files.len()) {
                    Ok(h) => (w, h),
                    Err(e) => {
                        send(ImportProgress::Error(e));
                        return;
                    }
                }
            }
        };

        let now = unix_now();
        new_entries.push(ManifestEntry {
            relative_path: rel,
            work_name: work_name.clone(),
            size_bytes: size,
            sha256: sha256.clone(),
            added_at_unix: now,
        });
        entries.push(ImageEntry {
            work_name: work_name.clone(),
            path: file.clone(),
            size_bytes: size,
            sha256,
            added_at_unix: now,
        });

        send(ImportProgress::FileDone { name: work_name });
    }

    let manifest = Manifest {
        version: MANIFEST_VERSION,
        images: new_entries,
    };
    if let Err(e) = save_manifest(&folder, &manifest) {
        send(ImportProgress::Error(e.to_string()));
        return;
    }

    send(ImportProgress::Done {
        imported: entries.len(),
        entries,
    });
}

fn hash_file(
    path: &Path,
    tx: &std::sync::mpsc::Sender<ImportProgress>,
    current: usize,
    total: usize,
) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;

    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    let size = file.metadata().map(|m| m.len()).unwrap_or(0);
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 4 * 1024 * 1024];
    let mut done: u64 = 0;
    let mut last_report = Instant::now();

    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    loop {
        let n = file.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        done += n as u64;

        if last_report.elapsed() >= Duration::from_millis(150) {
            let _ = tx.send(ImportProgress::HashingFile {
                current,
                total,
                name: name.clone(),
                bytes_done: done,
                bytes_total: size,
            });
            last_report = Instant::now();
        }
    }

    Ok(hex::encode(hasher.finalize()))
}

fn is_image_file(path: &Path) -> bool {
    match path.extension().and_then(|s| s.to_str()) {
        Some(ext) => matches!(ext.to_lowercase().as_str(), "img" | "iso" | "raw"),
        None => false,
    }
}

fn collect_images(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    collect_recursive(root, &mut out);
    out.sort();
    out
}

fn collect_recursive(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_recursive(&path, out);
        } else if is_image_file(&path) {
            out.push(path);
        }
    }
}

fn manifest_path(folder: &Path) -> PathBuf {
    folder.join(MANIFEST_NAME)
}

fn load_manifest(folder: &Path) -> Manifest {
    let p = manifest_path(folder);
    match fs::read_to_string(&p) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => Manifest::default(),
    }
}

fn save_manifest(folder: &Path, manifest: &Manifest) -> io::Result<()> {
    let p = manifest_path(folder);
    let s = serde_json::to_string_pretty(manifest)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    fs::write(p, s)
}

fn to_relative(folder: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(folder).ok()?;
    let mut s = String::new();
    for (i, comp) in rel.components().enumerate() {
        if i > 0 {
            s.push('/');
        }
        s.push_str(&comp.as_os_str().to_string_lossy());
    }
    Some(s)
}

fn default_work_name(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "image".to_string())
}