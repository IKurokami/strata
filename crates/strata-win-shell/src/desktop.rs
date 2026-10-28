// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    fs::{File, OpenOptions},
    io::{self, Write},
    sync::Mutex,
};
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        System::{Console::*, LibraryLoader::GetModuleHandleW},
        UI::{Shell::*, WindowsAndMessaging::*},
    },
    core::{PCWSTR, w},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Open,
    Console,
    ClearCache,
    Quit,
    Restart,
}

thread_local! {
    static EVENTS: RefCell<VecDeque<Event>> = const { RefCell::new(VecDeque::new()) };
    static TASKBAR_CREATED: Cell<u32> = const { Cell::new(0) };
}

/// Attach diagnostics to an existing terminal without creating a console window.
pub fn attach_parent_console() {
    // SAFETY: These are borrowed handles in this process. Attaching can replace the
    // standard-handle table; preserve explicit pipes/files without closing them.
    unsafe {
        let redirected: Vec<_> = [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE]
            .into_iter()
            .filter_map(|id| {
                let handle = GetStdHandle(id).ok()?;
                let kind = windows::Win32::Storage::FileSystem::GetFileType(handle);
                (kind == windows::Win32::Storage::FileSystem::FILE_TYPE_DISK
                    || kind == windows::Win32::Storage::FileSystem::FILE_TYPE_PIPE)
                    .then_some((id, handle))
            })
            .collect();
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
        for (id, handle) in redirected {
            let _ = SetStdHandle(id, handle);
        }
    }
}

/// Only consoles allocated here are hidden; an inherited terminal belongs to its caller.
#[derive(Default)]
pub struct Console {
    owned: bool,
}

const HISTORY_LIMIT: usize = 256 * 1024;

#[derive(Default)]
struct Diagnostics {
    history: VecDeque<u8>,
    console: Option<File>,
}

static DIAGNOSTICS: Mutex<Diagnostics> = Mutex::new(Diagnostics {
    history: VecDeque::new(),
    console: None,
});

/// Keep a bounded startup history and mirror logs to our own console independently
/// of Rust's cached standard streams and any launcher redirection.
pub fn write_diagnostics(bytes: &[u8]) -> bool {
    let mut diagnostics = DIAGNOSTICS
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if let Some(console) = diagnostics.console.as_mut() {
        let _ = console.write_all(bytes);
    }
    let retained = bytes.len().min(HISTORY_LIMIT);
    let discard = (diagnostics.history.len() + retained).saturating_sub(HISTORY_LIMIT);
    diagnostics.history.drain(..discard);
    diagnostics.history.extend(&bytes[bytes.len() - retained..]);
    let mut mode = CONSOLE_MODE::default();
    // SAFETY: This only queries the process's borrowed standard output handle.
    diagnostics.console.is_some()
        && unsafe {
            GetStdHandle(STD_OUTPUT_HANDLE)
                .is_ok_and(|handle| GetConsoleMode(handle, &mut mode).is_ok())
        }
}

impl Console {
    pub fn set_visible(&mut self, visible: bool) -> io::Result<()> {
        // SAFETY: These APIs operate on the current process's console only.
        unsafe {
            if visible && GetConsoleWindow().is_invalid() {
                let redirected: Vec<_> = [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE]
                    .into_iter()
                    .filter_map(|id| {
                        let handle = GetStdHandle(id).ok()?;
                        let kind = windows::Win32::Storage::FileSystem::GetFileType(handle);
                        (kind == windows::Win32::Storage::FileSystem::FILE_TYPE_DISK
                            || kind == windows::Win32::Storage::FileSystem::FILE_TYPE_PIPE)
                            .then_some((id, handle))
                    })
                    .collect();
                AllocConsole().map_err(io::Error::other)?;
                for (id, handle) in redirected {
                    let _ = SetStdHandle(id, handle);
                }
                self.owned = true;
                let _ = SetConsoleOutputCP(65001);
                let mut console = OpenOptions::new().write(true).open("CONOUT$")?;
                let mut diagnostics = DIAGNOSTICS
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                let history = diagnostics.history.make_contiguous();
                // Drop a possibly truncated UTF-8 prefix at the history boundary.
                let start = history
                    .iter()
                    .position(|byte| byte & 0xc0 != 0x80)
                    .unwrap_or(history.len());
                console.write_all(&history[start..])?;
                diagnostics.console = Some(console);
                let _ = SetConsoleTitleW(w!("Strata"));
                // The console is a diagnostic view, not a second application quit route.
                let menu = GetSystemMenu(GetConsoleWindow(), false);
                let _ = DeleteMenu(menu, SC_CLOSE, MF_BYCOMMAND);
                let _ = SetConsoleCtrlHandler(None, true);
            }
            if self.owned && IsWindowVisible(GetConsoleWindow()).as_bool() != visible {
                let _ = ShowWindow(GetConsoleWindow(), if visible { SW_SHOW } else { SW_HIDE });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;

const CALLBACK: u32 = WM_APP + 71;

unsafe extern "system" fn procedure(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_COMMAND {
        let event = match wparam.0 & 0xffff {
            1 => Some(Event::Open),
            2 => Some(Event::Console),
            3 => Some(Event::Quit),
            4 => Some(Event::ClearCache),
            _ => None,
        };
        if let Some(event) = event {
            EVENTS.with(|events| events.borrow_mut().push_back(event));
            return LRESULT(0);
        }
    }
    if TASKBAR_CREATED.with(|registered| registered.get() == message && message != 0) {
        EVENTS.with(|events| events.borrow_mut().push_back(Event::Restart));
        return LRESULT(0);
    }
    if message == CALLBACK {
        let event = match lparam.0 as u32 {
            WM_LBUTTONUP => Some(Event::Open),
            WM_RBUTTONUP | WM_CONTEXTMENU => {
                // SAFETY: The tray window owns this temporary popup; all handles are valid here.
                unsafe {
                    let menu = HMENU(GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut _);
                    let mut point = windows::Win32::Foundation::POINT::default();
                    let _ = GetCursorPos(&mut point);
                    let _ = SetForegroundWindow(hwnd);
                    let selected = TrackPopupMenu(
                        menu,
                        TPM_RETURNCMD | TPM_RIGHTBUTTON,
                        point.x,
                        point.y,
                        None,
                        hwnd,
                        None,
                    );
                    let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
                    match selected.0 {
                        1 => Some(Event::Open),
                        2 => Some(Event::Console),
                        3 => Some(Event::Quit),
                        4 => Some(Event::ClearCache),
                        _ => None,
                    }
                }
            }
            _ => None,
        };
        if let Some(event) = event {
            EVENTS.with(|events| events.borrow_mut().push_back(event));
        }
        return LRESULT(0);
    }
    // SAFETY: Forward untouched messages to the default window procedure.
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

/// Owned by the GTK thread, whose native pump also dispatches this hidden window.
pub struct Tray {
    window: HWND,
    menu: HMENU,
    data: NOTIFYICONDATAW,
    icon: HICON,
}

impl Tray {
    pub fn new(open: &str, console: &str, quit: &str, icon_png: &[u8]) -> io::Result<Self> {
        Self::create(open, console, quit, None, icon_png)
    }

    pub fn with_cache_action(
        open: &str,
        console: &str,
        quit: &str,
        clear_cache: &str,
        icon_png: &[u8],
    ) -> io::Result<Self> {
        Self::create(open, console, quit, Some(clear_cache), icon_png)
    }

    fn create(
        open: &str,
        console: &str,
        quit: &str,
        clear_cache: Option<&str>,
        icon_png: &[u8],
    ) -> io::Result<Self> {
        // SAFETY: Window class, menu and icon lifetimes are retained until Drop on this thread.
        unsafe {
            let instance = GetModuleHandleW(None).map_err(io::Error::other)?;
            let icon =
                CreateIconFromResourceEx(icon_png, true, 0x00030000, 32, 32, LR_DEFAULTCOLOR)
                    .map_err(io::Error::other)?;
            let class = WNDCLASSW {
                lpfnWndProc: Some(procedure),
                hInstance: instance.into(),
                lpszClassName: w!("Strata.Tray"),
                ..Default::default()
            };
            RegisterClassW(&class);
            let menu = match CreatePopupMenu() {
                Ok(menu) => menu,
                Err(error) => {
                    let _ = DestroyIcon(icon);
                    return Err(io::Error::other(error));
                }
            };
            for (id, label) in [
                (1, Some(open)),
                (2, Some(console)),
                (4, clear_cache),
                (3, Some(quit)),
            ] {
                let Some(label) = label else { continue };
                let label: Vec<u16> = label.encode_utf16().chain(Some(0)).collect();
                if let Err(error) = AppendMenuW(menu, MF_STRING, id, PCWSTR(label.as_ptr())) {
                    let _ = DestroyMenu(menu);
                    let _ = DestroyIcon(icon);
                    return Err(io::Error::other(error));
                }
            }
            let window = match CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                class.lpszClassName,
                w!("Strata"),
                WINDOW_STYLE::default(),
                0,
                0,
                0,
                0,
                None,
                None,
                Some(instance.into()),
                None,
            ) {
                Ok(window) => window,
                Err(error) => {
                    let _ = DestroyMenu(menu);
                    let _ = DestroyIcon(icon);
                    return Err(io::Error::other(error));
                }
            };
            SetWindowLongPtrW(window, GWLP_USERDATA, menu.0 as isize);
            let mut data = NOTIFYICONDATAW {
                cbSize: size_of::<NOTIFYICONDATAW>() as u32,
                hWnd: window,
                uID: 1,
                uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
                uCallbackMessage: CALLBACK,
                hIcon: icon,
                ..Default::default()
            };
            for (slot, unit) in data.szTip.iter_mut().zip("Strata".encode_utf16()) {
                *slot = unit;
            }
            TASKBAR_CREATED.with(|slot| slot.set(RegisterWindowMessageW(w!("TaskbarCreated"))));
            let tray = Self {
                window,
                menu,
                data,
                icon,
            };
            if !Shell_NotifyIconW(NIM_ADD, &tray.data).as_bool() {
                return Err(io::Error::last_os_error());
            }
            Ok(tray)
        }
    }

    pub fn receive(&self) -> Option<Event> {
        let event = EVENTS.with(|events| events.borrow_mut().pop_front());
        if event == Some(Event::Restart) {
            // SAFETY: Explorer lost its icon table; our callback window and icon are still owned.
            unsafe {
                let _ = Shell_NotifyIconW(NIM_ADD, &self.data);
            }
            return None;
        }
        event
    }

    pub fn set_console_checked(&self, enabled: bool) {
        // SAFETY: This menu is owned by this tray until Drop.
        unsafe {
            CheckMenuItem(
                self.menu,
                2,
                if enabled {
                    MF_CHECKED.0
                } else {
                    MF_UNCHECKED.0
                },
            );
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        // SAFETY: Remove the shell reference before destroying its callback window and menu.
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &self.data);
            SetWindowLongPtrW(self.window, GWLP_USERDATA, 0);
            let _ = DestroyMenu(self.menu);
            let _ = DestroyWindow(self.window);
            let _ = DestroyIcon(self.icon);
        }
        EVENTS.with(|events| events.borrow_mut().clear());
    }
}

/// Returns whether the user's Windows personalization settings prefer a dark app theme.
pub fn system_prefers_dark() -> bool {
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
    let subkey = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");
    for value_name in [w!("AppsUseLightTheme"), w!("SystemUsesLightTheme")] {
        let mut data = 0u32;
        let mut size = std::mem::size_of::<u32>() as u32;
        // SAFETY: `subkey` and `value_name` are null-terminated static wide strings,
        // and `data`/`size` point to valid stack storage of `u32`.
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                subkey,
                value_name,
                RRF_RT_REG_DWORD,
                None,
                Some((&mut data as *mut u32).cast()),
                Some(&mut size),
            )
        };
        if status.is_ok() {
            return data == 0;
        }
    }
    true
}
