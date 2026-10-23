// SPDX-License-Identifier: MIT

//! Native desktop input only. GTK key events continue through ui::keys.
use std::{
    cell::RefCell,
    io,
    sync::mpsc::{self, Receiver, SyncSender},
    thread::{self, JoinHandle},
};
use windows::Win32::{
    Foundation::{LPARAM, LRESULT, WPARAM},
    System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
    UI::{
        Input::KeyboardAndMouse::{
            GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
            SendInput, VK_CONTROL,
        },
        WindowsAndMessaging::*,
    },
};

const MENU_MASK: usize = 0x5354_5241;

struct Routing {
    keys: [bool; 256],
    consumed: bool,
}

impl Default for Routing {
    fn default() -> Self {
        Self {
            keys: [false; 256],
            consumed: false,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Decision {
    Pass,
    Consume,
    Activate,
}

impl Routing {
    fn event(&mut self, key: u32, down: bool, menu_mask: bool) -> Decision {
        if menu_mask || key >= 256 {
            return Decision::Pass;
        }
        if matches!(key, 0x10..=0x12 | 0x5b..=0x5c | 0xa0..=0xa5) {
            self.keys[key as usize] = down;
            return Decision::Pass;
        }
        if key != 0x45 {
            return Decision::Pass;
        }
        if self.consumed {
            if !down {
                self.consumed = false;
            }
            return Decision::Consume;
        }
        let win = self.keys[0x5b] || self.keys[0x5c];
        let other = [0x10, 0x11, 0x12, 0xa0, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5]
            .into_iter()
            .any(|key| self.keys[key]);
        if down && win && !other {
            self.consumed = true;
            Decision::Activate
        } else {
            Decision::Pass
        }
    }
}

thread_local! {
    static ROUTING: RefCell<Option<(Routing, SyncSender<()>)>> = const { RefCell::new(None) };
}

unsafe extern "system" fn callback(code: i32, message: WPARAM, parameter: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        // SAFETY: WH_KEYBOARD_LL supplies this structure for HC_ACTION only.
        let event = unsafe { &*(parameter.0 as *const KBDLLHOOKSTRUCT) };
        let down = matches!(message.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
        let up = matches!(message.0 as u32, WM_KEYUP | WM_SYSKEYUP);
        if down || up {
            let decision = ROUTING.with(|slot| {
                let Ok(mut slot) = slot.try_borrow_mut() else {
                    return Decision::Pass;
                };
                let Some((state, sender)) = slot.as_mut() else {
                    return Decision::Pass;
                };
                let decision = state.event(
                    event.vkCode,
                    down,
                    event.flags.contains(LLKHF_INJECTED) && event.dwExtraInfo == MENU_MASK,
                );
                if decision == Decision::Activate && sender.try_send(()).is_err() {
                    state.consumed = false;
                    return Decision::Pass;
                }
                decision
            });
            if decision == Decision::Activate {
                // Mask the bare Windows-key release so it does not open Start.
                // Only our mask bypasses routing; remapped/accessibility input still works.
                let input = [false, true].map(|up| INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: VK_CONTROL,
                            dwFlags: if up {
                                KEYEVENTF_KEYUP
                            } else {
                                Default::default()
                            },
                            dwExtraInfo: MENU_MASK,
                            ..Default::default()
                        },
                    },
                });
                // SAFETY: The two fully initialized keyboard inputs live for this call.
                unsafe {
                    SendInput(&input, size_of::<INPUT>() as i32);
                }
            }
            if decision != Decision::Pass {
                return LRESULT(1);
            }
        }
    }
    // SAFETY: Unhandled events must be passed down the hook chain unchanged.
    unsafe { CallNextHookEx(None, code, message, parameter) }
}

pub(super) struct Hook {
    thread: Option<JoinHandle<()>>,
    id: u32,
    pub events: Receiver<()>,
}

impl Hook {
    pub fn start() -> io::Result<Self> {
        let (sender, events) = mpsc::sync_channel(1);
        let (ready, receive) = mpsc::sync_channel(1);
        let thread = thread::Builder::new()
            .name("strata-desktop-keys".into())
            .spawn(move || {
                let mut state = Routing::default();
                for key in [0x5b, 0x5c, 0xa0, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5] {
                    // SAFETY: Query initial state outside the hook; callbacks update it before routing.
                    state.keys[key] = unsafe { GetAsyncKeyState(key as i32) } < 0;
                }
                ROUTING.with(|slot| *slot.borrow_mut() = Some((state, sender)));
                let setup = (|| {
                    // SAFETY: The current executable module remains loaded for the hook lifetime.
                    let module = unsafe { GetModuleHandleW(None) }?;
                    // SAFETY: This process retains the callback and runs its message loop until unhooked.
                    unsafe {
                        SetWindowsHookExW(WH_KEYBOARD_LL, Some(callback), Some(module.into()), 0)
                    }
                })();
                let hook = match setup {
                    Ok(hook) => hook,
                    Err(error) => {
                        let _ = ready.send(Err(io::Error::other(error)));
                        return;
                    }
                };
                let mut message = MSG::default();
                // SAFETY: PeekMessage creates this thread's queue before its ID is published.
                unsafe {
                    let _ = PeekMessageW(&mut message, None, 0, 0, PM_NOREMOVE);
                }
                // SAFETY: The current thread ID is valid until this thread exits.
                let _ = ready.send(Ok(unsafe { GetCurrentThreadId() }));
                // SAFETY: The writable MSG remains live and this dedicated thread owns the pump.
                while unsafe { GetMessageW(&mut message, None, 0, 0) }.0 > 0 {}
                // SAFETY: This is the hook installed by this thread and it is unhooked exactly once.
                let _ = unsafe { UnhookWindowsHookEx(hook) };
                ROUTING.with(|slot| *slot.borrow_mut() = None);
            })?;
        match receive.recv().map_err(io::Error::other)? {
            Ok(id) => Ok(Self {
                thread: Some(thread),
                id,
                events,
            }),
            Err(error) => {
                let _ = thread.join();
                Err(error)
            }
        }
    }
}

impl Drop for Hook {
    fn drop(&mut self) {
        // SAFETY: The hook thread published its ID after creating its message queue.
        let _ = unsafe { PostThreadMessageW(self.id, WM_QUIT, WPARAM(0), LPARAM(0)) };
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests;
