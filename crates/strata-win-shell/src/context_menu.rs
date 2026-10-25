//! Explorer's classic Shell menu, hosted only by the standalone shell helper.

use std::{io, os::windows::ffi::OsStrExt, path::Path};
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM},
        System::{
            Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemFree, CoUninitialize},
            LibraryLoader::GetModuleHandleW,
        },
        UI::{
            Shell::{
                BHID_SFUIObject, CMF_EXPLORE, CMF_NORMAL, CMINVOKECOMMANDINFO, IContextMenu,
                IContextMenu2, IContextMenu3, SHCreateShellItemArrayFromIDLists,
                SHParseDisplayName,
            },
            WindowsAndMessaging::*,
        },
    },
    core::{Interface, PCSTR, PCWSTR, w},
};

const FIRST_COMMAND: u32 = 1;
const LAST_COMMAND: u32 = 0x7fff;

struct Apartment;
impl Drop for Apartment {
    fn drop(&mut self) {
        // SAFETY: Created after successful COM initialization on this thread.
        unsafe { CoUninitialize() };
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
        unsafe { selection.BindToHandler(None, &BHID_SFUIObject) }.map_err(io::Error::other)
    })();
    for id in ids {
        // SAFETY: Each successful parse transferred one task-allocator PIDL to us.
        unsafe { CoTaskMemFree(Some(id.cast())) };
    }
    result
}

/// Allow the helper spawned by the foreground application to activate its menu.
pub fn allow_foreground(process_id: u32) {
    // SAFETY: This only grants foreground permission to the specified process.
    let _ = unsafe { AllowSetForegroundWindow(process_id) };
}

/// Show the installed Shell commands for the complete selection. Windows owns
/// positioning, scrolling, keyboard input, owner-draw and dynamic submenus.
/// Cancelling does not invoke a command. Extension code never runs inside GTK.
pub fn show(paths: &[impl AsRef<Path>]) -> io::Result<()> {
    validate_paths(paths)?;
    // SAFETY: The standalone helper owns COM initialization on this thread.
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
        .ok()
        .map_err(io::Error::other)?;
    let _apartment = Apartment;
    let context = selection_context(paths)?;
    let messages = MenuMessages {
        extended: context.cast().ok(),
        legacy: context.cast().ok(),
    };
    // SAFETY: Every handle and interface below remains live on this STA until
    // TrackPopupMenuEx and synchronous InvokeCommand have both completed.
    unsafe {
        let instance = GetModuleHandleW(None).map_err(io::Error::other)?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            lpszClassName: w!("StrataShellContextMenu"),
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
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
        SetWindowLongPtrW(owner.0, GWLP_USERDATA, &messages as *const _ as isize);
        let menu = Menu(CreatePopupMenu().map_err(io::Error::other)?);
        context
            .QueryContextMenu(
                menu.0,
                0,
                FIRST_COMMAND,
                LAST_COMMAND,
                CMF_NORMAL | CMF_EXPLORE,
            )
            .ok()
            .map_err(io::Error::other)?;
        let mut point = POINT::default();
        GetCursorPos(&mut point).map_err(io::Error::other)?;
        let _ = SetForegroundWindow(owner.0);
        let command = TrackPopupMenuEx(
            menu.0,
            (TPM_RETURNCMD | TPM_RIGHTBUTTON).0,
            point.x,
            point.y,
            owner.0,
            None,
        )
        .0 as u32;
        let _ = PostMessageW(Some(owner.0), WM_NULL, WPARAM(0), LPARAM(0));
        if command != 0 {
            if !(FIRST_COMMAND..=LAST_COMMAND).contains(&command) {
                return Err(io::Error::from_raw_os_error(87));
            }
            let invoke = CMINVOKECOMMANDINFO {
                cbSize: std::mem::size_of::<CMINVOKECOMMANDINFO>() as u32,
                hwnd: owner.0,
                lpVerb: PCSTR((command - FIRST_COMMAND) as usize as *const u8),
                nShow: SW_SHOWNORMAL.0,
                ..Default::default()
            };
            context.InvokeCommand(&invoke).map_err(io::Error::other)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
