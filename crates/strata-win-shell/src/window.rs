// SPDX-License-Identifier: MIT

use windows::Win32::{
    Foundation::{HWND, RECT},
    UI::WindowsAndMessaging::{
        GetWindowPlacement, IsIconic, SW_RESTORE, SW_SHOWMAXIMIZED, SW_SHOWNORMAL,
        SetForegroundWindow, SetWindowPlacement, ShowWindow, WINDOWPLACEMENT,
        WPF_RESTORETOMAXIMIZED,
    },
};

#[derive(Debug)]
pub struct DragState {
    pub origin: isize,
    pub foreground: isize,
    pub capture_on_caller: isize,
    pub foreground_pid: u32,
    pub caller_thread: u32,
}

/// Read native input state on the calling UI thread without changing activation.
pub fn drag_state(origin: isize) -> DragState {
    use windows::Win32::{
        System::Threading::GetCurrentThreadId,
        UI::{
            Input::KeyboardAndMouse::GetCapture,
            WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId},
        },
    };
    // SAFETY: These read-only APIs accept a nullable window and owned stack output.
    unsafe {
        let foreground = GetForegroundWindow();
        let mut foreground_pid = 0;
        let _ = GetWindowThreadProcessId(foreground, Some(&mut foreground_pid));
        DragState {
            origin,
            foreground: foreground.0 as isize,
            capture_on_caller: GetCapture().0 as isize,
            foreground_pid,
            caller_thread: GetCurrentThreadId(),
        }
    }
}

/// Work area of the monitor containing a client point, in client pixel coordinates.
pub fn popup_vertical_bounds(handle: isize, point: [i32; 2]) -> windows_core::Result<[i32; 2]> {
    use windows::Win32::{
        Foundation::POINT,
        Graphics::Gdi::{
            ClientToScreen, GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO,
            MonitorFromPoint,
        },
    };
    let window = HWND(handle as *mut _);
    let mut origin = POINT::default();
    // SAFETY: Windows validates this borrowed HWND; both output points are live.
    unsafe { ClientToScreen(window, &mut origin) }.ok()?;
    let pointer = POINT {
        x: origin.x.saturating_add(point[0]),
        y: origin.y.saturating_add(point[1]),
    };
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: The monitor is borrowed and the initialized output has the API's size.
    unsafe {
        GetMonitorInfoW(
            MonitorFromPoint(pointer, MONITOR_DEFAULTTONEAREST),
            &mut info,
        )
    }
    .ok()?;
    Ok([
        info.rcWork.top.saturating_sub(origin.y),
        info.rcWork.bottom.saturating_sub(origin.y),
    ])
}

/// Activate the existing browser after a user-initiated external open request.
pub fn foreground(handle: isize) {
    let window = HWND(handle as *mut _);
    // SAFETY: Windows validates the borrowed HWND; no GTK objects cross threads.
    unsafe {
        if IsIconic(window).as_bool() {
            let _ = ShowWindow(window, SW_RESTORE);
        }
        let _ = SetForegroundWindow(window);
    }
}

/// Match the process to the installed Start menu and desktop shortcuts.
pub fn set_application_id(id: &str) -> windows_core::Result<()> {
    let id = windows::core::HSTRING::from(id);
    // SAFETY: The identifier remains live for this synchronous Shell call.
    unsafe { windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID(&id) }
}

/// Normal workspace coordinates, independent of the window's minimized state.
#[derive(Clone, Copy)]
pub struct Placement {
    pub normal: [i32; 4],
    pub maximized: bool,
}

/// Capture a native HWND without changing its visibility or activation.
pub fn placement(handle: isize) -> windows_core::Result<Placement> {
    let mut placement = WINDOWPLACEMENT {
        length: size_of::<WINDOWPLACEMENT>() as u32,
        ..Default::default()
    };
    // SAFETY: The output is initialized and lives across this synchronous call.
    // Invalid or retired HWND values are rejected by Windows.
    unsafe { GetWindowPlacement(HWND(handle as *mut _), &mut placement) }?;
    let rect = placement.rcNormalPosition;
    Ok(Placement {
        normal: [rect.left, rect.top, rect.right, rect.bottom],
        maximized: placement.showCmd == SW_SHOWMAXIMIZED.0 as u32
            || placement.flags.contains(WPF_RESTORETOMAXIMIZED),
    })
}

/// Restore with the matching workspace-coordinate API. Windows moves an
/// off-screen placement onto an available monitor after display changes.
pub fn restore(handle: isize, saved: Placement) -> windows_core::Result<()> {
    let [left, top, right, bottom] = saved.normal;
    if right.checked_sub(left).is_none_or(|width| width <= 0)
        || bottom.checked_sub(top).is_none_or(|height| height <= 0)
    {
        return Err(windows_core::Error::from_hresult(
            windows::Win32::Foundation::E_INVALIDARG,
        ));
    }
    let placement = WINDOWPLACEMENT {
        length: size_of::<WINDOWPLACEMENT>() as u32,
        showCmd: if saved.maximized {
            SW_SHOWMAXIMIZED
        } else {
            SW_SHOWNORMAL
        }
        .0 as u32,
        rcNormalPosition: RECT {
            left,
            top,
            right,
            bottom,
        },
        ..Default::default()
    };
    // SAFETY: The placement is initialized and borrowed for this synchronous
    // call only; Windows validates the HWND and normalizes off-screen bounds.
    unsafe { SetWindowPlacement(HWND(handle as *mut _), &placement) }
}
