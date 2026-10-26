//! Прогресс в панели задач Windows (ITaskbarList3).
//! На не-Windows платформах — no-op.

#[cfg(windows)]
mod imp {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::System::Console::GetConsoleWindow;
    use windows::Win32::UI::Shell::{ITaskbarList3, TaskbarList, TBPF_NORMAL, TBPF_NOPROGRESS};

    pub struct Taskbar {
        list: Option<ITaskbarList3>,
        hwnd: HWND,
    }

    impl Taskbar {
        pub fn new() -> Self {
            unsafe {
                // CoInitializeEx может вернуть ошибку, если COM уже инициализирован
                // в другом режиме — это не критично, продолжаем.
                let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);

                let hwnd = GetConsoleWindow();
                let list: Option<ITaskbarList3> =
                    CoCreateInstance(&TaskbarList, None, CLSCTX_INPROC_SERVER).ok();

                if let Some(ref l) = list {
                    let _ = l.HrInit();
                }

                Self { list, hwnd }
            }
        }

        pub fn set(&self, done: u64, total: u64) {
            let Some(ref list) = self.list else {
                return;
            };
            unsafe {
                let _ = list.SetProgressState(self.hwnd, TBPF_NORMAL);
                let _ = list.SetProgressValue(self.hwnd, done, total.max(1));
            }
        }

        pub fn clear(&self) {
            let Some(ref list) = self.list else {
                return;
            };
            unsafe {
                let _ = list.SetProgressState(self.hwnd, TBPF_NOPROGRESS);
            }
        }
    }
}

#[cfg(not(windows))]
mod imp {
    pub struct Taskbar;
    impl Taskbar {
        pub fn new() -> Self {
            Taskbar
        }
        pub fn set(&self, _done: u64, _total: u64) {}
        pub fn clear(&self) {}
    }
}

pub use imp::Taskbar;