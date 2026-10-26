use std::ffi::CString;
use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct DiskInfo {
    pub number: u32,
    pub path: String,
    pub size_bytes: u64,
    pub is_usb: bool,
    pub bus_type: i32,
    pub vendor: String,
    pub product: String,
    pub serial: String,
}

pub fn enumerate_disks(_max: u32) -> Vec<DiskInfo> {
    let mut result = Vec::new();
    let sys_block = Path::new("/sys/block");
    let entries = match std::fs::read_dir(sys_block) {
        Ok(e) => e,
        Err(_) => return result,
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("sd") {
            continue;
        }
        let removable_path = entry.path().join("removable");
        let removable = std::fs::read_to_string(&removable_path)
            .map(|s| s.trim() == "1")
            .unwrap_or(false);
        if !removable {
            continue;
        }
        let size_path = entry.path().join("size");
        let size_sectors: u64 = std::fs::read_to_string(&size_path)
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0);
        let size_bytes = size_sectors * 512;
        let device_path = entry.path().join("device");
        let vendor = read_sys_attr(&device_path.join("vendor")).unwrap_or_default();
        let product = read_sys_attr(&device_path.join("model")).unwrap_or_default();
        let serial = read_sys_attr(&device_path.join("serial")).unwrap_or_default();
        let is_usb = check_usb_via_sysfs(&name);
        let number = if name.len() >= 3 {
            (name.as_bytes()[2] as u32).saturating_sub(b'a' as u32)
        } else {
            0
        };
        result.push(DiskInfo {
            number,
            path: format!("/dev/{}", name),
            size_bytes,
            is_usb,
            bus_type: if is_usb { 7 } else { 0 },
            vendor,
            product,
            serial,
        });
    }
    result.sort_by_key(|d| d.number);
    result
}

fn read_sys_attr(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn check_usb_via_sysfs(block_name: &str) -> bool {
    let device_path = format!("/sys/block/{}/device", block_name);
    let real_path = match std::fs::canonicalize(&device_path) {
        Ok(p) => p,
        Err(_) => return false,
    };
    real_path.to_string_lossy().contains("usb")
}

pub fn unmount_all_volumes_on_disk(disk_number: u32) -> io::Result<usize> {
    let device_name = number_to_device_name(disk_number);
    let mut count = 0;
    for i in 1..=16 {
        let full_path = format!("{}", device_name) + &i.to_string();
        if !Path::new(&full_path).exists() {
            continue;
        }
        if unmount_path(&full_path).is_ok() {
            count += 1;
        }
    }
    if unmount_path(&device_name).is_ok() {
        count += 1;
    }
    if count > 0 {
        std::thread::sleep(std::time::Duration::from_millis(300));
    }
    Ok(count)
}

pub fn number_to_device_name(number: u32) -> String {
    let letter = (b'a' + number as u8) as char;
    format!("/dev/sd{}", letter)
}

fn unmount_path(path: &str) -> io::Result<()> {
    let c_path = CString::new(path)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "bad path"))?;
    unsafe {
        let ret = libc::umount2(c_path.as_ptr(), 2);
        if ret != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

pub fn open_drive_write(disk_number: u32) -> io::Result<File> {
    let path = number_to_device_name(disk_number);
    OpenOptions::new()
        .write(true)
        .custom_flags(libc::O_SYNC)
        .open(&path)
}

pub fn open_drive_read(disk_number: u32) -> io::Result<File> {
    let path = number_to_device_name(disk_number);
    OpenOptions::new().read(true).open(&path)
}

pub fn update_disk_properties(file: &File) {
    unsafe {
        libc::ioctl(file.as_raw_fd(), 0x125f);
    }
}

pub fn flush_file(file: &File) -> io::Result<()> {
    file.sync_all()
}

pub fn set_current_thread_max_priority() -> io::Result<()> {
    unsafe {
        let param = libc::sched_param { sched_priority: 99 };
        let ret = libc::sched_setscheduler(0, libc::SCHED_FIFO, &param);
        if ret != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

pub fn set_disk_offline(_disk_number: u32, _offline: bool) -> io::Result<()> {
    Ok(())
}

pub fn sync_all() {
    unsafe {
        libc::sync();
    }
}
