# README FOR ANTIGRAVITY 🚀

> **Target:** Porting & Building **Mass USB Flasher (MUF)** for **macOS**
> **Architectures:** **x86_64-apple-darwin** (Intel Mac) first, followed by **aarch64-apple-darwin** (Apple Silicon M-series) & Universal 2 binary.
> **Date:** November 2026
> **Current Version:** `0.1.2`

---

## 1. Project Overview & Current Architecture

**Mass USB Flasher** is an ultra-fast, multi-device USB disk flasher and hardware stability benchmarking tool written in Rust (2021 edition).

* **Dual Binary Formats:**
  * `muf-gui`: GUI application built on `eframe` / `egui` (0.29).
  * `muf-tui`: Terminal dashboard built on `ratatui` (0.29).
* **Broadcast Core:**
  * Uses System76's lock-free `bus_writer` to read the image buffer (8 MB chunks) once and stream concurrently to all open drive file descriptors.
* **Benchmark Engine:**
  * Wipes MBR/GPT partition tables.
  * SplitMix64 pseudo-random patterned write/read tests with bitwise verification.
  * Real-time metrics (Avg, Min, Max, StdDev, Stability %) and polyline graph.
  * Sequential & Parallel queue modes.
  * Cool-down loops for thermal throttling / controller disconnect recovery.

---

## 2. Platform-Specific Abstraction Layers (What Needs macOS Implementation)

Currently, platform code is split into `cfg(windows)` and `cfg(not(windows))` (which is currently tailored for Linux).
To support macOS cleanly, we need to separate `cfg(target_os = "linux")` and `cfg(target_os = "macos")`.

### A. Device Enumeration & Identification (`src/device/`)
* **Current Linux implementation (`src/device/linux.rs`):**
  * Reads `/sys/block/sd*`, checks `/sys/block/sdX/removable == "1"`, resolves USB bus via sysfs canonicalization, gets vendor/model/serial from sysfs.
* **macOS Implementation (`src/device/macos.rs`):**
  * Use **IOKit** / **DiskArbitration** framework (or parse `diskutil list -plist` / `diskutil info -plist /dev/diskX`).
  * In pure Rust / FFI, `IOKit` (`IOBSDName`, `kIOCFPlugInLowLevelTypeID`, `kIOBlockStorageDeviceClass`) or the `core-foundation` + `iokit-sys` crates can query whether a disk is internal, removable, or attached over USB (`kIOPropertyPhysicalInterconnectTypeKey == "USB"`).
  * Filter out system APFS containers, synthesised disks, Time Machine drives, and internal NVMe (`disk0`, etc.).
  * Disks on macOS are formatted as `/dev/diskN` and `/dev/rdiskN` (see below for raw device access!).

### B. Raw Disk Access & High-Speed I/O (`src/engine/`)
* **Critical macOS Trait: `/dev/rdiskN` vs `/dev/diskN`:**
  * On macOS, `/dev/diskN` is a buffered block device — writing to it goes through the macOS unified buffer cache (UBC), which severely throttles speed (drops down to 10-20 MB/s) and may fail with buffer exhaustion.
  * **Always open the raw character device `/dev/rdiskN`** (e.g., `/dev/rdisk2` instead of `/dev/disk2`).
  * `/dev/rdiskN` provides direct unbuffered DMA access to the USB mass storage controller, delivering maximum hardware speed (100–400+ MB/s).
  * *Constraint:* All write/read operations on `/dev/rdiskN` must be aligned to physical sector boundaries (typically 512 or 4096 bytes). Our 1 MB / 4 MB / 8 MB chunk sizes naturally satisfy this.

### C. Unmounting & Volume Locking (`diskutil` / `DADiskUnmount`)
* macOS kernel automatically mounts FAT32/ExFAT/NTFS/APFS partitions as soon as a flash drive is plugged in.
* Before writing to `/dev/rdiskN`, all mounted volumes on that disk must be unmounted:
  * CLI option: `diskutil unmountDisk force /dev/diskN`
  * C/Rust API: `DADiskUnmount` from `DiskArbitration.framework`.
* Do NOT run `diskutil eject /dev/diskN` prior to writing, because ejecting turns off power to the drive or detaches it from the device tree. Only **unmount** all partitions.
* After flashing is finished, tell DiskArbitration or run `diskutil mountDisk /dev/diskN` (or leave unmounted).

### D. Partition Wiping (`BLKRRPART` equivalent on macOS)
* Linux uses `libc::ioctl(fd, 0x125f)` (`BLKRRPART`) to tell the kernel to reread partitions.
* On macOS, wiping the first 4 MB of `/dev/rdiskN` (MBR/GPT) and last 4 MB (Backup GPT) will trigger DiskArbitration uevents.
* Equivalent ioctl: `DKIOCCACHEFLUSH` (`ioctl(fd, DKIOCFLUSH)`) to flush drive write cache.
* Force re-read / probe:
  * In macOS, after writing raw partitions, run `diskutil quiet unmountDisk force /dev/diskN` or invoke `DADiskClaim`.

### E. Root / Administrator Privileges (macOS Elevation)
* Direct access to `/dev/rdiskN` requires `root` privileges.
* On macOS:
  * In terminal: run via `sudo ./muf-gui` or `sudo ./muf-tui`.
  * For `.app` bundles: standard macOS applications use an AppleScript / `AuthorizationExecuteWithPrivileges` / `SMJobBless` helper daemon or prompt with `osascript -e 'do shell script "..." with administrator privileges'`.
  * Update `src/common.rs` (`ensure_elevated`):
    * Linux checks `libc::geteuid() == 0`.
    * macOS also uses `libc::geteuid() == 0`. If `!= 0`, it can re-exec with `osascript` or prompt the user.

---

## 3. Required Code Changes

### Step 1: Update `Cargo.toml`
```toml
[target.'cfg(target_os = "macos")'.dependencies]
libc = "0.2"
# Optional for native IOKit / CoreFoundation if not using CLI wrappers:
# core-foundation = "0.10"
# mach2 = "0.4"
```

In `[dependencies.eframe]`, ensure macOS graphics backend is supported:
`eframe = { version = "0.29", default-features = false, features = ["default_fonts", "glow"] }`
On macOS, `glow` (OpenGL) or `wgpu` works well with Cocoa/Metal.

### Step 2: Device Module Organization
Refactor `src/device/mod.rs` & `src/engine/mod.rs`:
```rust
#[cfg(windows)]
pub mod windows;
#[cfg(windows)]
pub use windows::*;

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "linux")]
pub use linux::*;

#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "macos")]
pub use macos::*;
```

### Step 3: Implement `src/device/macos.rs` and `src/engine/macos.rs`
Key functions to implement:
* `enumerate_disks(max: u32) -> Vec<DiskInfo>`:
  * Method A (Fastest & robust): Execute `diskutil list -plist external physical` and parse XML via `plist` / `serde_json`.
  * Method B (Low-level native): Query `IOServiceMatching(kIOMediaClass)` via IOKit FFI, check `Ejectable == true` or `kIOPropertyPhysicalInterconnectTypeKey == "USB"`, extract BSD Name (`disk2`), Size, Model, Vendor.
  * Form path as `/dev/rdisk{N}` for raw I/O and store BSD name `/dev/disk{N}` for diskutil unmounting.
* `unmount_all_volumes_on_disk(disk_number: u32) -> io::Result<usize>`:
  * Command: `diskutil unmountDisk force /dev/disk{N}`.
* `open_drive_write(disk_number: u32) -> io::Result<File>`:
  * Open `/dev/rdisk{N}` with `OpenOptions::new().read(true).write(true)`.
  * Do NOT use `libc::O_SYNC` on macOS if it fails; macOS raw character devices `/dev/rdisk` bypass system cache by definition.
* `open_drive_read(disk_number: u32) -> io::Result<File>`:
  * Open `/dev/rdisk{N}` with `OpenOptions::new().read(true)`.
* `flush_file(file: &File) -> io::Result<()>`:
  * `file.sync_all()` or `libc::fcntl(fd, libc::F_FULLFSYNC)`.
* `sync_all()`:
  * `unsafe { libc::sync(); }`.
* `set_disk_offline(disk_number: u32, offline: bool) -> io::Result<()>`:
  * On macOS, no-op or re-run `diskutil unmountDisk force`.

### Step 4: Fix `src/taskbar.rs` and `src/common.rs`
* `src/taskbar.rs`: Already a no-op on `cfg(not(windows))`.
* `src/common.rs`:
  * Ensure `ensure_elevated` works on macOS:
    ```rust
    #[cfg(unix)]
    pub fn ensure_elevated(relaunch: bool) -> bool {
        unsafe {
            if libc::geteuid() == 0 {
                return true;
            }
        }
        // prompt or relaunch via sudo / osascript
        ...
    }
    ```

---

## 4. Cross-Compilation Setup (from Linux to macOS)

To cross-compile for macOS from Linux (Fedora/Ubuntu):

1. **Install OSXCROSS:**
   * Uses Clang and macOS SDK (e.g. MacOSX14.sdk).
   * Generates `x86_64-apple-darwin` and `aarch64-apple-darwin` cross-compiler toolchains.
2. **Add Rust Targets:**
   ```bash
   rustup target add x86_64-apple-darwin
   rustup target add aarch64-apple-darwin
   ```
3. **Build Command:**
   ```bash
   CC=x86_64-apple-darwin-clang cargo build --release --target x86_64-apple-darwin --bin muf-gui
   CC=x86_64-apple-darwin-clang cargo build --release --target x86_64-apple-darwin --bin muf-tui
   ```
4. **Alternative: Build directly on macOS runner (GitHub Actions):**
   * Using GitHub Actions `macos-13` (Intel x86_64) and `macos-14` (Apple Silicon M1):
   * This is the cleanest and most reliable way without managing macOS SDK licensing on Linux!
   * Use `lipo -create -output muf-gui-universal target/x86_64-apple-darwin/release/muf-gui target/aarch64-apple-darwin/release/muf-gui` to build a single Universal 2 fat binary.

---

## 5. Summary Checklist for the Agent

- [ ] Split `cfg(not(windows))` into `cfg(target_os = "linux")` and `cfg(target_os = "macos")`.
- [ ] Create `src/device/macos.rs` using `/dev/rdisk*` and external drive detection.
- [ ] Create `src/engine/macos.rs` with `diskutil unmountDisk force` and raw I/O.
- [ ] Test sector alignment for SplitMix64 1 MB chunks (already aligned to 4096 bytes).
- [ ] Add GitHub Actions CI workflow with `macos-13` (Intel) matrix build.
- [ ] Package into `.app` bundle with `Info.plist` and `icon.icns`.

*Ready for takeoff on macOS!* 🍏
