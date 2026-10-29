//! Shell command enumeration and invocation in the standalone helper.

use std::{collections::HashSet, io, os::windows::ffi::OsStrExt, path::Path};
use windows::{
    Win32::{
        Foundation::{ERROR_CLASS_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, WPARAM},
        System::{
            Com::{CoTaskMemFree, IDataObject},
            LibraryLoader::GetModuleHandleW,
            Ole::{OleFlushClipboard, OleInitialize, OleUninitialize},
        },
        UI::{
            Shell::{
                BHID_DataObject, BHID_SFUIObject, CMF_EXPLORE, CMF_NORMAL, CMF_SYNCCASCADEMENU,
                CMINVOKECOMMANDINFO, CMINVOKECOMMANDINFOEX, GCS_VERBW, IContextMenu, IContextMenu2,
                IContextMenu3, SEE_MASK_NOASYNC, SEE_MASK_UNICODE,
                SHCreateShellItemArrayFromIDLists, SHMultiFileProperties, SHParseDisplayName,
            },
            WindowsAndMessaging::*,
        },
    },
    core::{GUID, Interface, PCSTR, PCWSTR, PSTR, PWSTR, w},
};

const FIRST_COMMAND: u32 = 1;
const LAST_COMMAND: u32 = 0x7fff;
mod lifetime;

struct Apartment;
impl Drop for Apartment {
    fn drop(&mut self) {
        // SAFETY: Created after successful OLE initialization on this thread.
        unsafe { OleUninitialize() };
    }
}

struct Menu(HMENU);
impl Drop for Menu {
    fn drop(&mut self) {
        // SAFETY: This object uniquely owns the menu and all its submenus.
        let _ = unsafe { DestroyMenu(self.0) };
    }
}

struct Owner(HWND);
impl Drop for Owner {
    fn drop(&mut self) {
        // SAFETY: The helper owns this window on the creating STA thread.
        unsafe {
            SetWindowLongPtrW(self.0, GWLP_USERDATA, 0);
            let _ = DestroyWindow(self.0);
        }
    }
}

struct MenuMessages {
    extended: Option<IContextMenu3>,
    legacy: Option<IContextMenu2>,
}

unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: The pointer is installed only while the owning stack value lives;
    // Owner clears it before destroying the window.
    let messages = unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) as *const MenuMessages };
    if !messages.is_null()
        && matches!(
            message,
            WM_INITMENUPOPUP | WM_DRAWITEM | WM_MEASUREITEM | WM_MENUCHAR
        )
    {
        // SAFETY: The message loop and the context interfaces share this STA.
        let messages = unsafe { &*messages };
        if let Some(context) = &messages.extended {
            let mut result = LRESULT(0);
            // SAFETY: Forwarding the original message with writable result storage.
            if unsafe { context.HandleMenuMsg2(message, wparam, lparam, Some(&mut result)) }.is_ok()
            {
                return result;
            }
        } else if message != WM_MENUCHAR
            && let Some(context) = &messages.legacy
        {
            // SAFETY: Forwarding the original native menu message unchanged.
            if unsafe { context.HandleMenuMsg(message, wparam, lparam) }.is_ok() {
                return LRESULT(isize::from(message != WM_INITMENUPOPUP));
            }
        }
    }
    // SAFETY: Unhandled messages use the standard window procedure.
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

fn validate_paths(paths: &[impl AsRef<Path>]) -> io::Result<()> {
    if paths.is_empty()
        || paths.iter().any(|path| {
            let path = path.as_ref();
            !path.is_absolute() || path.as_os_str().encode_wide().any(|unit| unit == 0)
        })
    {
        return Err(io::Error::from_raw_os_error(87));
    }
    Ok(())
}

fn selection_context(paths: &[impl AsRef<Path>]) -> io::Result<IContextMenu> {
    selection_object(paths, &BHID_SFUIObject)
}

fn selection_object<T: Interface>(paths: &[impl AsRef<Path>], handler: &GUID) -> io::Result<T> {
    validate_paths(paths)?;
    let mut ids = Vec::with_capacity(paths.len());
    let result = (|| {
        for path in paths {
            let name: Vec<_> = path.as_ref().as_os_str().encode_wide().chain([0]).collect();
            let mut id = std::ptr::null_mut();
            // SAFETY: The UTF-16 name and output slot live through this call.
            unsafe { SHParseDisplayName(PCWSTR(name.as_ptr()), None, &mut id, 0, None) }
                .map_err(io::Error::other)?;
            ids.push(id);
        }
        let pointers: Vec<_> = ids.iter().map(|id| id.cast_const()).collect();
        // SAFETY: All absolute PIDLs remain allocated until the array is built.
        let selection =
            unsafe { SHCreateShellItemArrayFromIDLists(&pointers) }.map_err(io::Error::other)?;
        // SAFETY: Binding uses the Shell's selection-aware UI object on this STA.
        unsafe { selection.BindToHandler(None, handler) }.map_err(io::Error::other)
    })();
    for id in ids {
        // SAFETY: Each successful parse transferred one task-allocator PIDL to us.
        unsafe { CoTaskMemFree(Some(id.cast())) };
    }
    result
}

/// Show the Shell's selection-aware property sheet outside the GTK process.
pub fn properties(paths: &[impl AsRef<Path>]) -> io::Result<()> {
    // SAFETY: The standalone helper owns this thread's OLE apartment until the
    // native property sheet returns; Shell extensions never enter GTK.
    unsafe { OleInitialize(None) }.map_err(io::Error::other)?;
    let _apartment = Apartment;
    let _lifetime = lifetime::ShellLifetime::new()?;
    let data: IDataObject = selection_object(paths, &BHID_DataObject)?;
    // SAFETY: The Shell-created data object contains the selected absolute PIDLs
    // and remains alive for the duration of the native property sheet.
    unsafe { SHMultiFileProperties(&data, 0) }.map_err(io::Error::other)
}

/// Allow the helper to activate a selected command's dialog or application.
pub fn allow_foreground(process_id: u32) {
    // SAFETY: This only grants foreground permission to the specified process.
    let _ = unsafe { AllowSetForegroundWindow(process_id) };
}

/// A menu item exported to the GTK process without native drawing or handles.
#[derive(Clone, Debug)]
pub struct Entry {
    pub label: String,
    pub command: Option<u32>,
    pub verb: String,
    pub enabled: bool,
    pub checked: bool,
    pub separator: bool,
    pub submenu: bool,
    pub children: Vec<Entry>,
}

/// Owns the original Shell command table and its apartment until invocation or
/// cancellation. Construct and use only on the standalone helper's STA thread.
pub struct Session {
    menu: Menu,
    owner: Owner,
    _messages: Box<MenuMessages>,
    context: IContextMenu,
    allowed: HashSet<u32>,
    paths: Vec<std::path::PathBuf>,
    _apartment: Apartment,
}

impl Session {
    pub fn new(paths: &[impl AsRef<Path>]) -> io::Result<Self> {
        validate_paths(paths)?;
        // SAFETY: The helper owns this apartment until Session is dropped.
        unsafe { OleInitialize(None) }.map_err(io::Error::other)?;
        let apartment = Apartment;
        let context = selection_context(paths)?;
        let messages = Box::new(MenuMessages {
            extended: context.cast().ok(),
            legacy: context.cast().ok(),
        });
        // SAFETY: All handles and interfaces belong to this STA. The boxed message
        // target stays at a stable address and is dropped after the owner window.
        unsafe {
            let instance = GetModuleHandleW(None).map_err(io::Error::other)?;
            let class = WNDCLASSW {
                lpfnWndProc: Some(window_proc),
                hInstance: instance.into(),
                lpszClassName: w!("StrataShellContextMenu"),
                ..Default::default()
            };
            if RegisterClassW(&class) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
                return Err(io::Error::last_os_error());
            }
            let owner = Owner(
                CreateWindowExW(
                    WS_EX_TOOLWINDOW,
                    class.lpszClassName,
                    w!("Strata"),
                    WS_POPUP,
                    0,
                    0,
                    0,
                    0,
                    None,
                    None,
                    Some(instance.into()),
                    None,
                )
                .map_err(io::Error::other)?,
            );
            SetWindowLongPtrW(
                owner.0,
                GWLP_USERDATA,
                messages.as_ref() as *const _ as isize,
            );
            let menu = Menu(CreatePopupMenu().map_err(io::Error::other)?);
            context
                .QueryContextMenu(
                    menu.0,
                    0,
                    FIRST_COMMAND,
                    LAST_COMMAND,
                    CMF_NORMAL | CMF_EXPLORE | CMF_SYNCCASCADEMENU,
                )
                .ok()
                .map_err(io::Error::other)?;
            Ok(Self {
                menu,
                owner,
                _messages: messages,
                context,
                allowed: HashSet::new(),
                paths: paths
                    .iter()
                    .map(|path| path.as_ref().to_path_buf())
                    .collect(),
                _apartment: apartment,
            })
        }
    }

    /// Initialize dynamic Shell submenus and export a bounded, display-free tree.
    pub fn snapshot(&mut self) -> io::Result<Vec<Entry>> {
        let mut remaining = 4096;
        let entries = self.read_menu(self.menu.0, 0, 0, &mut remaining)?;
        self.allowed.clear();
        fn commands(entries: &[Entry], enabled: bool, result: &mut HashSet<u32>) {
            for entry in entries {
                let enabled = enabled && entry.enabled;
                if enabled && let Some(command) = entry.command {
                    result.insert(command);
                }
                commands(&entry.children, enabled, result);
            }
        }
        commands(&entries, true, &mut self.allowed);
        Ok(entries)
    }

    fn read_menu(
        &self,
        menu: HMENU,
        position: u32,
        depth: usize,
        remaining: &mut usize,
    ) -> io::Result<Vec<Entry>> {
        if depth > 12 {
            return Err(io::Error::other("Shell menu nesting limit exceeded"));
        }
        // SAFETY: This live menu belongs to the session; the original interfaces
        // receive initialization through the owner window on their creating STA.
        unsafe {
            SendMessageW(
                self.owner.0,
                WM_INITMENUPOPUP,
                Some(WPARAM(menu.0 as usize)),
                Some(LPARAM(position as isize)),
            )
        };
        // SAFETY: The menu remains live throughout this traversal.
        let count = unsafe { GetMenuItemCount(Some(menu)) };
        if count < 0 {
            return Err(io::Error::last_os_error());
        }
        let mut entries = Vec::new();
        for position in 0..count as u32 {
            *remaining = remaining
                .checked_sub(1)
                .ok_or_else(|| io::Error::other("Shell menu item limit exceeded"))?;
            let mut info = MENUITEMINFOW {
                cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32,
                fMask: MIIM_FTYPE | MIIM_STATE | MIIM_ID | MIIM_SUBMENU | MIIM_STRING,
                ..Default::default()
            };
            // SAFETY: The first call obtains the required UTF-16 label length.
            unsafe { GetMenuItemInfoW(menu, position, true, &mut info) }
                .map_err(io::Error::other)?;
            if info.cch > 8192 {
                return Err(io::Error::other("Shell menu label limit exceeded"));
            }
            let mut label = vec![0u16; info.cch as usize + 1];
            info.dwTypeData = PWSTR(label.as_mut_ptr());
            info.cch = label.len() as u32;
            // SAFETY: The buffer matches the reported length and outlives the call.
            unsafe { GetMenuItemInfoW(menu, position, true, &mut info) }
                .map_err(io::Error::other)?;
            let separator = info.fType.contains(MFT_SEPARATOR);
            let submenu = !info.hSubMenu.is_invalid();
            let command =
                (!separator && !submenu && (FIRST_COMMAND..=LAST_COMMAND).contains(&info.wID))
                    .then_some(info.wID);
            let mut label = String::from_utf16_lossy(
                &label[..label.iter().position(|&c| c == 0).unwrap_or(label.len())],
            );
            let mut canonical = String::new();
            if let Some(command) = command {
                let mut verb = [0u16; 512];
                // SAFETY: GCS_VERBW expects a UTF-16 buffer despite the PSTR ABI.
                if unsafe {
                    self.context.GetCommandString(
                        (command - FIRST_COMMAND) as usize,
                        GCS_VERBW,
                        None,
                        PSTR(verb.as_mut_ptr().cast()),
                        verb.len() as u32,
                    )
                }
                .is_ok()
                {
                    canonical = String::from_utf16_lossy(
                        &verb[..verb.iter().position(|&c| c == 0).unwrap_or(verb.len())],
                    );
                }
            }
            if label.is_empty() {
                label.clone_from(&canonical);
            }
            let children = if submenu {
                self.read_menu(info.hSubMenu, position, depth + 1, remaining)?
            } else {
                Vec::new()
            };
            entries.push(Entry {
                label,
                command,
                verb: canonical,
                enabled: (info.fState & (MFS_DISABLED | MFS_GRAYED)).0 == 0,
                checked: info.fState.contains(MFS_CHECKED),
                separator,
                submenu,
                children,
            });
        }
        Ok(entries)
    }

    /// Invoke only an enabled command from this session's exported tree.
    pub fn invoke(&self, command: u32) -> io::Result<()> {
        if !self.allowed.contains(&command) {
            return Err(io::Error::from_raw_os_error(87));
        }
        let mut verb = [0u16; 512];
        // SAFETY: The canonical Unicode verb uses the original command offset.
        let verb = if unsafe {
            self.context.GetCommandString(
                (command - FIRST_COMMAND) as usize,
                GCS_VERBW,
                None,
                PSTR(verb.as_mut_ptr().cast()),
                verb.len() as u32,
            )
        }
        .is_ok()
        {
            String::from_utf16_lossy(
                &verb[..verb.iter().position(|&c| c == 0).unwrap_or(verb.len())],
            )
        } else {
            String::new()
        };
        if verb.eq_ignore_ascii_case("properties")
            || verb.eq_ignore_ascii_case("windows.properties")
        {
            return properties(&self.paths);
        }
        invoke_command(&self.context, self.owner.0, command)?;
        if verb.eq_ignore_ascii_case("copy") || verb.eq_ignore_ascii_case("cut") {
            // SAFETY: Persist the explicitly selected clipboard operation before
            // this source process exits, while its OLE apartment is still live.
            unsafe { OleFlushClipboard() }.map_err(io::Error::other)?;
        }
        Ok(())
    }

    /// Service STA callbacks while waiting for the user's GTK menu selection.
    pub fn pump_messages(&self) {
        let mut message = MSG::default();
        // SAFETY: Message dispatch and all Shell objects share this STA thread.
        unsafe {
            while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }
}

fn invoke_command(context: &IContextMenu, owner: HWND, command: u32) -> io::Result<()> {
    let invoke = CMINVOKECOMMANDINFOEX {
        cbSize: std::mem::size_of::<CMINVOKECOMMANDINFOEX>() as u32,
        // The helper owns the apartment: launching asynchronously can leave
        // a handler without its COM objects as soon as InvokeCommand returns.
        fMask: SEE_MASK_UNICODE | SEE_MASK_NOASYNC,
        hwnd: owner,
        lpVerb: PCSTR((command - FIRST_COMMAND) as usize as *const u8),
        lpVerbW: PCWSTR((command - FIRST_COMMAND) as usize as *const u16),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    // SAFETY: The extended structure includes the documented base prefix and
    // remains live with the original context and owner throughout invocation.
    unsafe {
        context
            .InvokeCommand((&invoke as *const CMINVOKECOMMANDINFOEX).cast::<CMINVOKECOMMANDINFO>())
    }
    .map_err(io::Error::other)
}

#[cfg(test)]
mod tests;
