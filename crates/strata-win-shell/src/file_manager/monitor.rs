// SPDX-License-Identifier: MIT
use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
use windows::{
    Win32::{
        Foundation::{CloseHandle, WAIT_OBJECT_0},
        System::{
            Registry::*,
            Threading::{CreateEventW, WaitForSingleObject},
        },
    },
    core::PCWSTR,
};

pub struct RegistryMonitor(Arc<AtomicBool>);
impl Drop for RegistryMonitor {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

/// This watch belongs to the GUI process; it never handles keyboard input.
pub fn watch_changes(changed: impl Fn() + Send + 'static) -> io::Result<RegistryMonitor> {
    let stop = Arc::new(AtomicBool::new(false));
    let cancelled = stop.clone();
    thread::Builder::new()
        .name("shell-registry-notify".into())
        .spawn(move || {
            let path: Vec<u16> = "Software\\Classes\0".encode_utf16().collect();
            let mut key = HKEY::default();
            // SAFETY: Terminated read-only key name and initialized handle output.
            if unsafe {
                RegOpenKeyExW(
                    HKEY_CURRENT_USER,
                    PCWSTR(path.as_ptr()),
                    None,
                    KEY_NOTIFY | KEY_WOW64_64KEY,
                    &mut key,
                )
            }
            .is_err()
            {
                changed();
                return;
            }
            // SAFETY: This thread owns the unnamed auto-reset event until cleanup below.
            let event = match unsafe { CreateEventW(None, false, false, None) } {
                Ok(event) => event,
                Err(_) => {
                    // SAFETY: Opening the key succeeded; no event or notification exists yet.
                    unsafe {
                        let _ = RegCloseKey(key);
                    }
                    changed();
                    return;
                }
            };
            let arm = || {
                // SAFETY: Live key and event are owned by this thread; notification is asynchronous.
                unsafe {
                    RegNotifyChangeKeyValue(
                        key,
                        true,
                        REG_NOTIFY_CHANGE_NAME | REG_NOTIFY_CHANGE_LAST_SET,
                        Some(event),
                        true,
                    )
                }
                .is_ok()
            };
            if arm() {
                while !cancelled.load(Ordering::Acquire) {
                    // SAFETY: Event remains valid until the loop exits.
                    if unsafe { WaitForSingleObject(event, 500) } == WAIT_OBJECT_0 {
                        if !arm() {
                            break;
                        }
                        thread::sleep(Duration::from_millis(200));
                        changed();
                    }
                }
            }
            // SAFETY: Closing the key cancels notifications before the owned event is closed.
            unsafe {
                let _ = RegCloseKey(key);
            }
            // SAFETY: The worker no longer uses the event.
            unsafe {
                let _ = CloseHandle(event);
            }
        })?;
    Ok(RegistryMonitor(stop))
}
