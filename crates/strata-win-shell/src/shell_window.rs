// SPDX-License-Identifier: MIT
//! Shell window registration for SHOpenFolderAndSelectItems callers such as Chrome.

use std::{
    cell::{Cell, RefCell},
    os::windows::ffi::OsStrExt,
    path::PathBuf,
    rc::Rc,
    sync::mpsc::{self, Receiver, Sender, SyncSender},
    thread,
    time::Duration,
};
use windows::{
    Win32::{
        Foundation::*,
        System::{Com::*, Ole::*, Variant::*},
        UI::{Shell::*, WindowsAndMessaging::*},
    },
    core::{BSTR, IUnknownImpl, Interface, implement},
};

mod interfaces;
#[cfg(test)]
mod tests;

pub enum Event {
    Registered,
    Selection {
        folder: PathBuf,
        files: Vec<PathBuf>,
    },
    Show,
    Failed(String),
}

enum Command {
    Navigate(PathBuf),
    Stop,
}

pub struct Registration {
    commands: Sender<Command>,
    events: Receiver<Event>,
}

impl Registration {
    pub fn new(window: isize, folder: PathBuf) -> std::io::Result<Self> {
        let (commands, receive) = mpsc::channel();
        let (events, output) = mpsc::sync_channel(64);
        thread::Builder::new()
            .name("strata-shell-window".into())
            .spawn(move || {
                if let Err(error) = run(window, folder, receive, events.clone()) {
                    let _ = events.try_send(Event::Failed(error.to_string()));
                }
            })?;
        Ok(Self {
            commands,
            events: output,
        })
    }

    pub fn navigate(&self, folder: PathBuf) {
        let _ = self.commands.send(Command::Navigate(folder));
    }

    pub fn receive(&self) -> Option<Event> {
        self.events.try_recv().ok()
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        // Do not block GTK on an outgoing COM call during window teardown.
        let _ = self.commands.send(Command::Stop);
    }
}

struct State {
    window: isize,
    folder: RefCell<PathBuf>,
    selected: RefCell<Vec<PathBuf>>,
    selection_pending: Cell<bool>,
    events: SyncSender<Event>,
}

// The default free-threaded marshaler would expose this STA-owned state to
// concurrent Shell callers. Marshal every callback onto the owning apartment.
#[implement(
    IDispatch,
    IServiceProvider,
    IShellBrowser,
    IShellView,
    IWebBrowserApp,
    Agile = false
)]
struct View {
    state: Rc<State>,
}

fn run(
    window: isize,
    folder: PathBuf,
    commands: Receiver<Command>,
    events: SyncSender<Event>,
) -> windows::core::Result<()> {
    // SAFETY: This private thread exclusively owns the apartment and its message pump.
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.ok()?;
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            // SAFETY: Balances successful initialization on this thread.
            unsafe { CoUninitialize() };
        }
    }
    let _apartment = Apartment;
    // SAFETY: ShellWindows is an OS local server; no GTK objects cross this apartment.
    let shell: IShellWindows =
        unsafe { CoCreateInstance(&ShellWindows, None, CLSCTX_LOCAL_SERVER) }?;
    let state = Rc::new(State {
        window,
        folder: RefCell::new(folder),
        selected: RefCell::new(Vec::new()),
        selection_pending: Cell::new(false),
        events,
    });
    let object: IDispatch = View {
        state: state.clone(),
    }
    .into();
    let mut cookie = register(&shell, &object, &state)?;
    let _ = state.events.try_send(Event::Registered);
    let result = (|| {
        loop {
            loop {
                match commands.try_recv() {
                    Ok(Command::Navigate(folder)) => {
                        // SAFETY: The cookie belongs to this shell registration.
                        unsafe { shell.Revoke(cookie) }?;
                        cookie = 0;
                        *state.folder.borrow_mut() = folder;
                        state.selected.borrow_mut().clear();
                        state.selection_pending.set(false);
                        cookie = register(&shell, &object, &state)?;
                        let _ = state.events.try_send(Event::Registered);
                    }
                    Ok(Command::Stop) | Err(mpsc::TryRecvError::Disconnected) => return Ok(()),
                    Err(mpsc::TryRecvError::Empty) => break,
                }
            }
            let mut message = MSG::default();
            // SAFETY: This STA thread owns the queue and the initialized message buffer.
            while unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }.as_bool() {
                if message.message == WM_QUIT {
                    return Ok(());
                }
                // SAFETY: PeekMessage initialized this thread's message.
                unsafe {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            if state.selection_pending.replace(false) {
                let event = Event::Selection {
                    folder: state.folder.borrow().clone(),
                    files: state.selected.borrow().clone(),
                };
                if state.events.try_send(event).is_err() {
                    state.selection_pending.set(true);
                }
            }
            thread::sleep(Duration::from_millis(4));
        }
    })();
    if cookie != 0 {
        // SAFETY: Release registration before the object and apartment are dropped.
        let _ = unsafe { shell.Revoke(cookie) };
    }
    result
}

fn register(
    shell: &IShellWindows,
    object: &IDispatch,
    state: &State,
) -> windows::core::Result<i32> {
    let folder = state.folder.borrow().clone();
    if !folder.is_absolute() {
        return Err(windows::core::Error::from_hresult(E_INVALIDARG));
    }
    let location = VARIANT::from(BSTR::from_wide(
        &folder.as_os_str().encode_wide().collect::<Vec<_>>(),
    ));
    let root = VARIANT::default();
    let window = HWND(state.window as *mut _);
    // SAFETY: Only the thread ID of a live GTK HWND is read here.
    let thread = unsafe { GetWindowThreadProcessId(window, None) };
    if thread == 0 {
        return Err(windows::core::Error::from_hresult(E_ABORT));
    }
    // SAFETY: Location variants and dispatch remain live for each synchronous call.
    let pending = unsafe { shell.RegisterPending(thread as i32, &location, &root, SWC_BROWSER) }?;
    // SAFETY: The handle is the same window/thread used by RegisterPending.
    match unsafe { shell.Register(object, state.window as i32, SWC_BROWSER) } {
        Ok(cookie) => {
            if cookie != pending {
                // SAFETY: A distinct pending entry must not survive final registration.
                let _ = unsafe { shell.Revoke(pending) };
            }
            Ok(cookie)
        }
        Err(error) => {
            // SAFETY: Roll back only the pending entry created above.
            let _ = unsafe { shell.Revoke(pending) };
            Err(error)
        }
    }
}

fn selected_path(
    folder: &std::path::Path,
    pidl: *const Common::ITEMIDLIST,
) -> windows::core::Result<PathBuf> {
    // SAFETY: COM marshals the borrowed PIDL for the lifetime of SelectItem.
    let absolute: windows::core::Result<IShellItem> = unsafe { SHCreateItemFromIDList(pidl) };
    if let Ok(item) = absolute
        && let Ok(path) = crate::item_path(&item)
        && path
            .parent()
            .is_some_and(|parent| crate::file_manager::same_path(parent, folder))
    {
        return Ok(path);
    }
    let name = windows::core::HSTRING::from(folder.as_os_str());
    let mut parent = std::ptr::null_mut();
    // SAFETY: Shell allocates a fully qualified parent PIDL, freed below.
    unsafe { SHParseDisplayName(&name, None, &mut parent, 0, None) }?;
    // SAFETY: Both PIDLs are live. ILCombine allocates an independent absolute PIDL.
    let combined = unsafe { ILCombine(Some(parent), Some(pidl)) };
    // SAFETY: Release only the allocation returned by SHParseDisplayName.
    unsafe { CoTaskMemFree(Some(parent.cast())) };
    if combined.is_null() {
        return Err(windows::core::Error::from_hresult(E_OUTOFMEMORY));
    }
    // SAFETY: Combined PIDL remains live while the shell item is created.
    let item: windows::core::Result<IShellItem> = unsafe { SHCreateItemFromIDList(combined) };
    // SAFETY: Release only the independent ILCombine allocation.
    unsafe { CoTaskMemFree(Some(combined.cast())) };
    item.and_then(|item| {
        crate::item_path(&item).map_err(|_| windows::core::Error::from_hresult(E_FAIL))
    })
}
