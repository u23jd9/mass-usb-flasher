use std::ffi::c_void;
use std::fs::File;
use std::io;
use std::os::windows::io::AsRawHandle;
use std::thread;
use std::time::Duration;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FindFirstVolumeW, FindNextVolumeW, FindVolumeClose, FlushFileBuffers,
    FILE_ATTRIBUTE_NORMAL, FILE_FLAG_NO_BUFFERING, FILE_SHARE_READ, FILE_SHARE_WRITE,
    OPEN_EXISTING,
};
use windows::Win32::System::Ioctl::{
    DISK_ATTRIBUTE_OFFLINE, FSCTL_DISMOUNT_VOLUME, FSCTL_LOCK_VOLUME,
    IOCTL_DISK_SET_DISK_ATTRIBUTES, IOCTL_DISK_UPDATE_PROPERTIES,
    IOCTL_STORAGE_GET_DEVICE_NUMBER, SET_DISK_ATTRIBUTES, STORAGE_DEVICE_NUMBER,
};
use windows::Win32::System::IO::DeviceIoControl;

const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;

/// Открывает физический диск для записи.
///
/// FILE_FLAG_NO_BUFFERING критичен для raw-записи: гарантирует,
/// что данные уходят напрямую на носитель, минуя системный кэш.
/// FILE_FLAG_WRITE_THROUGH НЕ используем — он замедляет запись
/// в разы (Windows ждёт подтверждения от носителя после каждой
/// операции).
pub fn open_drive_write(number: u32) -> io::Result<File> {
    let path = format!(r"\\.\PhysicalDrive{}", number);
    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();

    unsafe {
        let handle: HANDLE = CreateFileW(
            PCWSTR(wide.as_ptr()),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL | FILE_FLAG_NO_BUFFERING,
            None,
        )
        .map_err(|e| io::Error::from_raw_os_error(e.code().0))?;

        use std::os::windows::io::FromRawHandle;
        Ok(File::from_raw_handle(handle.0 as *mut _))
    }
}

/// Принудительно размонтирует ВСЕ тома на диске `disk_number`.
///
/// Использует FSCTL_DISMOUNT_VOLUME с retry — он пытается размонтировать
/// том, даже если его кто-то держит. FSCTL_LOCK_VOLUME применяется
/// как мягкая попытка (ошибки игнорируются).
pub fn unmount_all_volumes_on_disk(disk_number: u32) -> io::Result<usize> {
    let mut count = 0;
    let mut volume_buf = [0u16; 1024];

    unsafe {
        let find_handle = match FindFirstVolumeW(&mut volume_buf) {
            Ok(h) => h,
            Err(_) => return Ok(0),
        };

        loop {
            let vol_name = pwstr_to_string(&volume_buf);
            let trimmed = vol_name.trim_end_matches('\\');

            if !trimmed.is_empty() {
                if let Ok(h) = open_volume(trimmed) {
                    if volume_belongs_to_disk(&h, disk_number).unwrap_or(false) {
                        let _ = lock_volume_with_retry(&h);
                        if dismount_volume_with_retry(&h).is_ok() {
                            count += 1;
                        }
                    }
                    let _ = CloseHandle(h);
                }
            }

            match FindNextVolumeW(find_handle, &mut volume_buf) {
                Ok(()) => continue,
                Err(_) => break,
            }
        }

        let _ = FindVolumeClose(find_handle);
    }

    if count > 0 {
        thread::sleep(Duration::from_millis(500));
    }
    Ok(count)
}

/// Принудительно переводит физический диск в состояние Offline или Online.
///
/// Offline заставляет Windows «забыть» о томах на диске и перестать
/// блокировать прямой доступ к секторам через \\.\PhysicalDriveN.
/// Это ключевой шаг для обхода ACCESS_DENIED при записи.
///
/// Persist = false — состояние не сохраняется после перезагрузки,
/// что безопасно: если процесс упадёт, диск сам вернётся в Online.
pub fn set_disk_offline(disk_number: u32, offline: bool) -> io::Result<()> {
    let file = open_drive_write(disk_number)?;
    let handle = HANDLE(file.as_raw_handle() as *mut c_void);
    let mut returned = 0u32;

    let mut attrs = SET_DISK_ATTRIBUTES::default();
    attrs.Version = std::mem::size_of::<SET_DISK_ATTRIBUTES>() as u32;
    attrs.Persist = false.into();
    if offline {
        attrs.Attributes = DISK_ATTRIBUTE_OFFLINE as u64;
    } else {
        attrs.Attributes = 0;
    }
    attrs.AttributesMask = DISK_ATTRIBUTE_OFFLINE as u64;

    unsafe {
        DeviceIoControl(
            handle,
            IOCTL_DISK_SET_DISK_ATTRIBUTES,
            Some(&attrs as *const _ as *const c_void),
            std::mem::size_of::<SET_DISK_ATTRIBUTES>() as u32,
            None,
            0,
            Some(&mut returned),
            None,
        )
        .map_err(|e| io::Error::from_raw_os_error(e.code().0))?;

        // Обновляем свойства, чтобы Windows немедленно увидела изменения.
        let _ = DeviceIoControl(
            handle,
            IOCTL_DISK_UPDATE_PROPERTIES,
            None,
            0,
            None,
            0,
            Some(&mut returned),
            None,
        );
    }

    // Даём Windows время «переварить» смену состояния.
    thread::sleep(Duration::from_millis(200));
    Ok(())
}

fn pwstr_to_string(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

fn open_volume(path: &str) -> io::Result<HANDLE> {
    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let h = CreateFileW(
            PCWSTR(wide.as_ptr()),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
        .map_err(|e| io::Error::from_raw_os_error(e.code().0))?;
        Ok(h)
    }
}

/// Определяет, лежит ли том на указанном физическом диске.
fn volume_belongs_to_disk(volume: &HANDLE, disk_number: u32) -> io::Result<bool> {
    let mut info = STORAGE_DEVICE_NUMBER::default();
    let mut returned = 0u32;

    unsafe {
        DeviceIoControl(
            *volume,
            IOCTL_STORAGE_GET_DEVICE_NUMBER,
            None,
            0,
            Some(&mut info as *mut _ as *mut c_void),
            std::mem::size_of::<STORAGE_DEVICE_NUMBER>() as u32,
            Some(&mut returned),
            None,
        )
        .map_err(|e| io::Error::from_raw_os_error(e.code().0))?;
    }

    Ok(info.DeviceNumber == disk_number)
}

/// FSCTL_LOCK_VOLUME с retry. Мягкая попытка — если не сработает,
/// dismount всё равно попробует.
fn lock_volume_with_retry(volume: &HANDLE) -> io::Result<()> {
    let mut last_err = None;
    for attempt in 0..3 {
        match lock_volume(volume) {
            Ok(()) => return Ok(()),
            Err(e) => {
                last_err = Some(e);
                thread::sleep(Duration::from_millis(100 * (attempt + 1)));
            }
        }
    }
    Err(last_err.unwrap_or_else(|| io::Error::new(io::ErrorKind::Other, "lock failed")))
}

/// FSCTL_DISMOUNT_VOLUME с retry — принудительное размонтирование.
fn dismount_volume_with_retry(volume: &HANDLE) -> io::Result<()> {
    let mut last_err = None;
    for attempt in 0..5 {
        match dismount_volume(volume) {
            Ok(()) => return Ok(()),
            Err(e) => {
                last_err = Some(e);
                thread::sleep(Duration::from_millis(150 * (attempt + 1)));
            }
        }
    }
    Err(last_err.unwrap_or_else(|| io::Error::new(io::ErrorKind::Other, "dismount failed")))
}

fn lock_volume(volume: &HANDLE) -> io::Result<()> {
    let mut returned = 0u32;
    unsafe {
        DeviceIoControl(
            *volume,
            FSCTL_LOCK_VOLUME,
            None,
            0,
            None,
            0,
            Some(&mut returned),
            None,
        )
        .map_err(|e| io::Error::from_raw_os_error(e.code().0))?;
    }
    Ok(())
}

fn dismount_volume(volume: &HANDLE) -> io::Result<()> {
    let mut returned = 0u32;
    unsafe {
        DeviceIoControl(
            *volume,
            FSCTL_DISMOUNT_VOLUME,
            None,
            0,
            None,
            0,
            Some(&mut returned),
            None,
        )
        .map_err(|e| io::Error::from_raw_os_error(e.code().0))?;
    }
    Ok(())
}

/// Сообщает Windows, что таблица разделов изменилась.
pub fn update_disk_properties(file: &File) {
    let handle = HANDLE(file.as_raw_handle() as *mut c_void);
    let mut returned = 0u32;
    unsafe {
        let _ = DeviceIoControl(
            handle,
            IOCTL_DISK_UPDATE_PROPERTIES,
            None,
            0,
            None,
            0,
            Some(&mut returned),
            None,
        );
    }
}

/// Сбрасывает системный кэш файла на физический носитель.
/// Пригодится на этапе верификации.
pub fn flush_file(file: &File) -> io::Result<()> {
    let handle = HANDLE(file.as_raw_handle() as *mut c_void);
    unsafe {
        FlushFileBuffers(handle).map_err(|e| io::Error::from_raw_os_error(e.code().0))?;
    }
    Ok(())
}