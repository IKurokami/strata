use std::{
    io,
    sync::atomic::{AtomicI32, Ordering},
};
use windows::{
    Win32::UI::{
        Shell::{SHCreateThreadRef, SHSetInstanceExplorer, SHSetThreadRef},
        WindowsAndMessaging::*,
    },
    core::IUnknown,
};

/// The standalone helper must outlive asynchronous Shell property-sheet threads.
pub(super) struct ShellLifetime {
    count: Box<AtomicI32>,
    reference: Option<IUnknown>,
    baseline: i32,
}

impl ShellLifetime {
    pub(super) fn new() -> io::Result<Self> {
        let count = Box::new(AtomicI32::new(0));
        let mut reference = None;
        // SAFETY: The aligned reference counter has a stable allocation that is
        // retained until every Shell reference is released. Shell uses interlocked updates.
        unsafe { SHCreateThreadRef(count.as_ptr(), &mut reference) }.map_err(io::Error::other)?;
        // SAFETY: This helper's STA owns the registration and keeps its reference alive.
        unsafe { SHSetThreadRef(reference.as_ref()) }.map_err(io::Error::other)?;
        // SAFETY: SHCreateThreadRef supplies the Shell's free-threaded IUnknown;
        // only the standalone process registers a process lifetime reference.
        unsafe { SHSetInstanceExplorer(reference.as_ref()) };
        let baseline = count.load(Ordering::Acquire);
        Ok(Self {
            count,
            reference,
            baseline,
        })
    }

    fn busy(&self) -> bool {
        self.count.load(Ordering::Acquire) > self.baseline
    }
}

impl Drop for ShellLifetime {
    fn drop(&mut self) {
        while self.busy() {
            let mut message = MSG::default();
            // SAFETY: Message dispatch stays on the creating STA, allowing
            // marshaled callbacks to finish while Shell threads own the reference.
            unsafe {
                while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            if self.busy() {
                // SAFETY: Waiting on this thread's message queue with no handle array.
                let _ = unsafe {
                    MsgWaitForMultipleObjectsEx(None, 50, QS_ALLINPUT, MWMO_INPUTAVAILABLE)
                };
            }
        }
        // SAFETY: The helper has no remaining hosted Shell work. Clear the
        // borrowed registrations before releasing the last owned reference/counter.
        unsafe { SHSetInstanceExplorer(None::<&IUnknown>) };
        // SAFETY: Clear the current STA's borrowed thread reference while it is alive.
        let _ = unsafe { SHSetThreadRef(None::<&IUnknown>) };
        drop(self.reference.take());
    }
}

#[cfg(test)]
mod tests;
