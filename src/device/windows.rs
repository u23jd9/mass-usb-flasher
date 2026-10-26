use std::ffi::c_void;
use std::fs::{File, OpenOptions};
use std::io;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::ptr;

use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::FileSystem::BusTypeUsb;
use windows::Win32::System::Ioctl::{
    GET_LENGTH_INFORMATION, IOCTL_DISK_GET_LENGTH_INFO,
    IOCTL_STORAGE_QUERY_PROPERTY, PropertyStandardQuery,
    STORAGE_DEVICE_DESCRIPTOR, STORAGE_PROPERTY_QUERY,
    StorageDeviceProperty,
};
use windows::Win32::System::IO::DeviceIoControl;

const FILE_SHARE_READ: u32 = 0x0000_0001;
const FILE_SHARE_WRITE: u32 = 0x0000_0002;

#[allow(dead_code)]
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

pub fn open_drive_read(number: u32) -> io::Result<File> {
    let path = format!(r"\\.\PhysicalDrive{}", number);
    OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .open(&path)
}

fn handle_of(file: &File) -> HANDLE {
    HANDLE(file.as_raw_handle() as *mut c_void)
}

fn query_length(file: &File) -> io::Result<u64> {
    let mut info = GET_LENGTH_INFORMATION::default();
    let mut returned = 0u32;
    unsafe {
        DeviceIoControl(
            handle_of(file),
            IOCTL_DISK_GET_LENGTH_INFO,
            None,
            0,
            Some(&mut info as *mut _ as *mut c_void),
            std::mem::size_of::<GET_LENGTH_INFORMATION>() as u32,
            Some(&mut returned),
            None,
        )
        .map_err(|e| io::Error::from_raw_os_error(e.code().0))?;
    }
    Ok(info.Length as u64)
}

fn query_device_descriptor(file: &File) -> io::Result<Vec<u8>> {
    let query = STORAGE_PROPERTY_QUERY {
        PropertyId: StorageDeviceProperty,
        QueryType: PropertyStandardQuery,
        AdditionalParameters: [0],
    };

    let mut buf = vec![0u8; 1024];
    let mut returned = 0u32;

    unsafe {
        DeviceIoControl(
            handle_of(file),
            IOCTL_STORAGE_QUERY_PROPERTY,
            Some(&query as *const _ as *const c_void),
            std::mem::size_of::<STORAGE_PROPERTY_QUERY>() as u32,
            Some(buf.as_mut_ptr() as *mut c_void),
            buf.len() as u32,
            Some(&mut returned),
            None,
        )
        .map_err(|e| io::Error::from_raw_os_error(e.code().0))?;
    }

    buf.truncate(returned as usize);
    Ok(buf)
}

fn read_cstr_at(buf: &[u8], offset: u32) -> String {
    if offset == 0 {
        return String::new();
    }
    let start = offset as usize;
    if start >= buf.len() {
        return String::new();
    }
    let end = buf[start..]
        .iter()
        .position(|&b| b == 0)
        .map(|i| start + i)
        .unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[start..end]).trim().to_string()
}

fn parse_descriptor(buf: &[u8]) -> (bool, i32, String, String, String) {
    if buf.len() < std::mem::size_of::<STORAGE_DEVICE_DESCRIPTOR>() {
        return (false, 0, String::new(), String::new(), String::new());
    }

    let desc: STORAGE_DEVICE_DESCRIPTOR =
        unsafe { ptr::read_unaligned(buf.as_ptr() as *const STORAGE_DEVICE_DESCRIPTOR) };

    let is_usb = desc.BusType == BusTypeUsb;
    let bus_type = desc.BusType.0;
    let vendor = read_cstr_at(buf, desc.VendorIdOffset);
    let product = read_cstr_at(buf, desc.ProductIdOffset);
    let serial = read_cstr_at(buf, desc.SerialNumberOffset);

    (is_usb, bus_type, vendor, product, serial)
}

pub fn enumerate_disks(max: u32) -> Vec<DiskInfo> {
    let mut result = Vec::new();

    for n in 0..max {
        let file = match open_drive_read(n) {
            Ok(f) => f,
            Err(e) => {
                let code = e.raw_os_error().unwrap_or(0);
                if code == 5 {
                    result.push(DiskInfo {
                        number: n,
                        path: format!(r"\\.\PhysicalDrive{}", n),
                        size_bytes: 0,
                        is_usb: false,
                        bus_type: -1,
                        vendor: String::new(),
                        product: String::new(),
                        serial: String::new(),
                    });
                }
                continue;
            }
        };

        let size_bytes = query_length(&file).unwrap_or(0);

        let (is_usb, bus_type, vendor, product, serial) =
            match query_device_descriptor(&file) {
                Ok(buf) => parse_descriptor(&buf),
                Err(_) => (false, 0, String::new(), String::new(), String::new()),
            };

        result.push(DiskInfo {
            number: n,
            path: format!(r"\\.\PhysicalDrive{}", n),
            size_bytes,
            is_usb,
            bus_type,
            vendor,
            product,
            serial,
        });
    }

    result
}