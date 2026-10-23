// SPDX-License-Identifier: MIT

use super::{Registration, hotkey::Hook, lock};
use crate::activation::{self, Server};
use std::{
    io,
    path::Path,
    process::Command,
    thread,
    time::{Duration, Instant},
};

const SCOPE: &str = "Strata.FileManager.Shortcut";

pub(super) fn stop() -> io::Result<()> {
    if activation::forward(SCOPE, b"stop").is_err() {
        return Ok(());
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    while activation::forward(SCOPE, b"ping").is_ok() {
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Shell helper did not stop",
            ));
        }
        thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}

pub(super) fn ensure(proxy: &Path) -> io::Result<()> {
    if activation::forward(SCOPE, b"ping").is_ok() {
        return Ok(());
    }
    let mut child = Command::new(proxy)
        .arg("--watch")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if activation::forward(SCOPE, b"ping").is_ok() {
            return Ok(());
        }
        if let Some(status) = child.try_wait()? {
            return Err(io::Error::other(format!("Shell helper exited: {status}")));
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Shell helper did not become ready",
            ));
        }
        thread::sleep(Duration::from_millis(10));
    }
}

/// Keep the native desktop shortcut active while this per-user registration exists.
/// This process loads no GTK runtime. Removal of the target repairs the journal.
pub fn watch() -> io::Result<()> {
    if activation::forward(SCOPE, b"ping").is_ok() {
        return Ok(());
    }
    let registration = Registration::default();
    {
        let _guard = lock::acquire()?;
        if registration.available_target()?.is_none() {
            super::notify();
            return Ok(());
        }
    }
    let hook = Hook::start()?;
    let server = match Server::bind(SCOPE) {
        Ok(server) => server,
        Err(error) => {
            drop(hook);
            return if activation::forward(SCOPE, b"ping").is_ok() {
                Ok(())
            } else {
                Err(error)
            };
        }
    };
    let mut next_check = Instant::now();
    loop {
        while let Some(message) = server.receive() {
            if message == b"stop" {
                return Ok(());
            }
        }
        let activate = hook.events.try_recv().is_ok();
        if activate || Instant::now() >= next_check {
            let target = {
                let _guard = lock::acquire()?;
                let owner = registration.owner()?;
                let target = registration.available_target()?;
                if owner.is_some() && target.is_none() {
                    super::notify();
                }
                target
            };
            let Some(target) = target else {
                return Ok(());
            };
            let forwarded = activate && registration.activate_running();
            if activate && !forwarded && Command::new(&target).spawn().is_err() {
                let _guard = lock::acquire()?;
                registration.disable(&target)?;
                super::notify();
                let windows = std::env::var_os("SystemRoot")
                    .ok_or_else(|| io::Error::other("Missing Windows directory"))?;
                Command::new(Path::new(&windows).join("explorer.exe")).spawn()?;
                return Ok(());
            }
            next_check = Instant::now() + Duration::from_millis(500);
        }
        thread::sleep(Duration::from_millis(10));
    }
}
