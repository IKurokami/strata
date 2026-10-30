// SPDX-License-Identifier: MIT
//! Compatibility with an already-running v1 helper. V2 never starts a hook.
use crate::activation;
use std::{
    io, thread,
    time::{Duration, Instant},
};

const SCOPE: &str = "Strata.FileManager.Shortcut";
pub(super) fn running() -> bool {
    activation::forward(SCOPE, b"ping").is_ok()
}
pub(super) fn stop() -> io::Result<()> {
    if activation::forward(SCOPE, b"stop").is_err() {
        return Ok(());
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    while running() {
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Legacy shell helper did not stop",
            ));
        }
        thread::sleep(Duration::from_millis(20));
    }
    Ok(())
}
/// Old Run entries may invoke the new binary during an upgrade. Never install a hook.
pub fn watch() -> io::Result<()> {
    stop()
}
