# Mass USB Flasher (MUF) ⚡

A high-performance, lightweight cross-platform utility written in **Rust** for concurrent raw disk flashing and hardware stress-testing / benchmarking of USB flash drives (`.img`, `.iso`, `.raw`).

**English** • [Русская версия](README.ru.md)

---

Designed for commercial and industrial deployment workflows: replicating batches of USB drives (5+ devices simultaneously) with guaranteed integrity, blazing-fast speeds, built-in drive health/thermal diagnostics, and an intuitive UI.

---

## 🚀 Features & Highlights

* **Dual Interface from a Single Codebase:**
  * **GUI (`muf-gui`):** Modern graphical interface built with `eframe`/`egui`. Completely standalone, zero external dependencies, native HiDPI adaptive scaling, Light/Dark theme toggle, and responsive layout.
  * **TUI (`muf-tui`):** Ultra-fast terminal dashboard built with `ratatui` (~2.0 MB).
* **Parallel Broadcast Writing without Disk Bottlenecks:**
  Powered by System76's lock-free `bus_writer`. The source disk image is read into an 8 MB buffer once and broadcast concurrently to all opened flash drives.
* **⚡ Built-in Hardware Stress Benchmark & Stability Tester:**
  A dedicated tab designed to weed out counterfeit, slow, or thermally unstable flash drives before mass production:
  * **Partition Table Wipe (MBR/GPT):** Automatically wipes the first and last 4 MB of each drive (MBR, primary and backup secondary GPT headers) and forcefully unmounts OS volume locks before testing.
  * **Sequential Write & Read Tests:** Generates deterministic pseudo-random non-compressible blocks (SplitMix64) to prevent controller hardware compression, paired with bitwise verification on read.
  * **Real-time Stability Metrics & Sparklines:** Computes Average, Minimum, and Maximum speeds, standard deviation (StdDev), and a percentage stability score (`Stability %`) with real-time polyline graphing.
  * **Queue Scheduling (Sequential vs Parallel):** **Sequential** mode (default) tests drives one by one to safeguard USB hubs against 5V power drops and host controller resets; **Parallel** runs stress-tests across all drives at once.
  * **Detailed Controller Thermal Diagnostics:** Detects flash controller resets and dropouts caused by thermal overload, inserts cooling pauses, and resumes tests automatically without false failures.
* **Rock-solid Windows Support:**
  * Solves `ACCESS_DENIED` errors on live drives by unmounting hidden/EFI volumes (`FindFirstVolumeW`) and taking disks **offline** via `IOCTL_DISK_SET_DISK_ATTRIBUTES`.
  * Maximum I/O throughput with `FILE_FLAG_NO_BUFFERING` bypassing `WRITE_THROUGH`.
  * Native Windows Taskbar progress integration via COM `ITaskbarList3`.
  * Embedded UAC manifest requesting Administrator privileges automatically.
* **First-class Linux Support:**
  * Device discovery and hotplug via `/sys/block` and `sysfs`.
  * Direct synchronous I/O via `O_SYNC` to `/dev/sdX` with clean partition unmounting (`umount2`) and `systemd-udevd` race condition prevention.
* **One-Click Batch Reset (Ready / Idle):**
  Instantly prepare for the next batch with GUI button **`[ 🔄 Ready for Next Batch ]`** or TUI key **`[R]` / `[Enter]`** without restarting the application.
* **Image Catalog & Background SHA-256:**
  * Built-in image library (`%USERPROFILE%\.muf\catalog.json` / `~/.config/muf/catalog.json`) with custom labels.
  * Batch folder imports via `.muf-images.json` metadata manifests.
  * Non-blocking background SHA-256 calculation with live progress and throughput metrics.
* **Bilingual UI:**
  Instant 1-click toggle between **English** and **Russian** (`L` key).

---

## ⚡ How the USB Benchmark Works

The **Benchmark** tab isolates flaky, fake, or overheating USB flash drives through a 5-step automated pipeline:

1. **Step 1: Partition Table Erasure (MBR & GPT)**
   * Zeroes out sector 0 and the secondary backup GPT at the end of the physical media, flushes kernel caches, and releases volume handles.
2. **Step 2: Sequential Write Test**
   * Writes configured sample sizes (from 256 MB up to Full Disk) using pseudo-random patterned data. Measures sustained write speed and detects SLC-cache exhaustion drops.
3. **Step 3: Cache Flush & Cool-down Settle Pause**
   * Enforces raw NAND buffer synchronization and provides a 1.5-second settle window to stabilize power rails.
4. **Step 4: Sequential Read & Bitwise Verification**
   * Reads data back at full interface speed, verifying each 64-bit chunk against the SplitMix64 generator to detect bit flips, bad sectors, or counterfeit capacity.
5. **Step 5: Stability Verdict & Telemetry**
   * Computes coefficient of speed variance (`Stability %`) and assigns a human-readable verdict (*Excellent*, *Good*, *Thermal Throttle*, or *Controller Dropout*).

---

## 📦 Releases & Binaries

> [!NOTE]
> **Platform Testing & Stability Policy:**
> * Bleeding-edge features and new versions are always developed and verified primarily on **Linux (Ubuntu / Fedora)**.
> * Version **[v0.1.1](https://github.com/u23jd9/mass-usb-flasher/releases/tag/v0.1.1)** is thoroughly tested and verified on both Linux and **Windows**.
> * New releases and pre-release builds (v0.1.2-beta+) undergo active evolution; unexpected bugs or edge-case quirks may occur.

Pre-compiled standalone binaries are available on [GitHub Releases](https://github.com/u23jd9/mass-usb-flasher/releases):

| Binary | Platform | Interface | Description |
|---|---|---|---|
| `muf-gui` | Linux (x86_64) | GUI | Native graphical application (Wayland / X11) with Flashing & Benchmark tabs |
| `muf-tui` | Linux (x86_64) | TUI | Terminal console dashboard |
| `muf-gui.exe` | Windows (x64) | GUI | Windowed standalone binary with embedded icon & UAC manifest |
| `muf-tui.exe` | Windows (x64) | TUI | Console terminal application |

---

## 🛠 Building from Source

Requires the **Rust** toolchain (1.80+).

```bash
# Clone the repository
git clone https://github.com/u23jd9/mass-usb-flasher.git
cd mass-usb-flasher

# Build both TUI and GUI in release mode
cargo build --release
```

Binaries will be output to `target/release/`:
* `muf-gui`
* `muf-tui`

---

## ⌨ Hotkeys (TUI)

| Key | Action |
|---|---|
| `I` | Open Image Catalog |
| `S` | Start concurrent flashing (when Ready) |
| `R` / `Enter` | Reset status to Ready / Waiting for next batch |
| `+` / `-` | Adjust Target drive count |
| `L` | Toggle language (English / Russian) |
| `H` | Help overlay |
| `Q` / `Ctrl+C` | Quit |

---

## 📄 License & Author

Crafted by: **Telegram: [@delyoself](https://t.me/delyoself)**  
License: MIT / Apache-2.0
