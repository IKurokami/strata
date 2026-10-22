// SPDX-License-Identifier: MIT

use std::{
    ffi::{OsStr, OsString},
    io::{self, Write},
    os::windows::{ffi::OsStrExt, fs::MetadataExt},
    path::{Path, PathBuf},
    process::Command,
};
use windows::Win32::{
    Globalization::{CSTR_EQUAL, CompareStringOrdinal},
    UI::Shell::{SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify},
};

mod registry;
use registry::Value;

mod hotkey;
mod lock;
mod watcher;
pub use watcher::watch;

const VERB: &str = "Strata.Open";
const RUN_VALUE: &str = "StrataFileManager";
const CLASSES: [&str; 2] = ["Directory", "Drive"];

struct Registration {
    classes: String,
    state: String,
    run: String,
}

impl Default for Registration {
    fn default() -> Self {
        Self {
            classes: r"Software\Classes".into(),
            state: r"Software\Strata\FileManager".into(),
            run: r"Software\Microsoft\Windows\CurrentVersion\Run".into(),
        }
    }
}

pub(crate) fn same_path(first: &Path, second: &Path) -> bool {
    let first: Vec<u16> = first.as_os_str().encode_wide().collect();
    let second: Vec<u16> = second.as_os_str().encode_wide().collect();
    // SAFETY: Both length-delimited UTF-16 buffers remain valid for the call.
    unsafe { CompareStringOrdinal(&first, &second, true) == CSTR_EQUAL }
}

fn notify() {
    // SAFETY: Association-change notification takes no item pointers.
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
}

impl Registration {
    fn set_activation(&self, activation: Option<(&str, &[u8])>) -> io::Result<()> {
        use windows::Win32::System::Registry::REG_BINARY;

        let value = match activation {
            Some((scope, request))
                if !scope.is_empty()
                    && scope.len() <= 32 * 1024
                    && !request.is_empty()
                    && request.len() <= 60 * 1024 =>
            {
                let mut data = (scope.len() as u32).to_le_bytes().to_vec();
                data.extend_from_slice(scope.as_bytes());
                data.extend_from_slice(request);
                Some(Value {
                    kind: REG_BINARY,
                    data,
                })
            }
            Some(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Invalid shell activation request",
                ));
            }
            None => None,
        };
        if registry::get(&self.state, "Activation")? != value {
            registry::set(&self.state, "Activation", value.as_ref())?;
            registry::flush(&self.state)?;
        }
        Ok(())
    }

    fn activate_running(&self) -> bool {
        self.forward_running(None)
    }

    fn forward_running(&self, message: Option<&[u8]>) -> bool {
        use windows::Win32::System::Registry::REG_BINARY;

        let Ok(Some(value)) = registry::get(&self.state, "Activation") else {
            return false;
        };
        if value.kind != REG_BINARY || value.data.len() < 4 {
            return false;
        }
        let length =
            u32::from_le_bytes(value.data[..4].try_into().expect("activation header")) as usize;
        if length == 0 || length > 32 * 1024 || length > value.data.len() - 4 {
            return false;
        }
        let Ok(scope) = std::str::from_utf8(&value.data[4..4 + length]) else {
            return false;
        };
        crate::activation::forward(scope, message.unwrap_or(&value.data[4 + length..])).is_ok()
    }

    fn shell(&self, class: &str) -> String {
        format!(r"{}\{class}\shell", self.classes)
    }
    fn verb(&self, class: &str) -> String {
        format!(r"{}\{VERB}", self.shell(class))
    }

    fn owner(&self) -> io::Result<Option<PathBuf>> {
        Ok(registry::get(&self.state, "Executable")?
            .and_then(|value| value.os_string())
            .map(PathBuf::from))
    }

    fn command(&self) -> io::Result<Option<Value>> {
        registry::get(&self.state, "Command")
    }

    fn available_target(&self) -> io::Result<Option<PathBuf>> {
        let Some(executable) = self.owner()? else {
            return Ok(None);
        };
        if executable.is_file() {
            return Ok(Some(executable));
        }
        self.disable(&executable)?;
        Ok(None)
    }

    fn enabled(&self, executable: &Path) -> io::Result<bool> {
        if !self
            .owner()?
            .is_some_and(|owner| same_path(&owner, executable))
        {
            return Ok(false);
        }
        let expected = self.command()?;
        let verb = Some(Value::text(OsStr::new(VERB))?);
        for class in CLASSES {
            if registry::get(&self.shell(class), "")? != verb
                || registry::get(&format!(r"{}\command", self.verb(class)), "")? != expected
            {
                return Ok(false);
            }
        }
        Ok(expected.is_some()
            && registry::get(&self.run, RUN_VALUE)? == registry::get(&self.state, "WatchCommand")?)
    }

    fn disable(&self, executable: &Path) -> io::Result<()> {
        if !self
            .owner()?
            .is_some_and(|owner| same_path(&owner, executable))
        {
            return Ok(());
        }
        let command = self
            .command()?
            .ok_or_else(|| io::Error::other("Missing file-manager command backup"))?;
        let verb = Some(Value::text(OsStr::new(VERB))?);
        for class in CLASSES {
            let own_command = registry::get(&format!(r"{}\command", self.verb(class)), "")?
                .as_ref()
                == Some(&command);
            if own_command {
                if registry::get(&self.shell(class), "")? == verb {
                    let backup = registry::get(&self.state, class)?
                        .ok_or_else(|| io::Error::other("Missing file-manager default backup"))?;
                    registry::set(&self.shell(class), "", backup.original()?.as_ref())?;
                }
                registry::remove_tree(&self.verb(class))?;
            }
        }
        if let Some(expected) = registry::get(&self.state, "WatchCommand")?
            && registry::get(&self.run, RUN_VALUE)?.as_ref() == Some(&expected)
        {
            let backup = registry::get(&self.state, "RunBackup")?
                .ok_or_else(|| io::Error::other("Missing startup command backup"))?;
            registry::set(&self.run, RUN_VALUE, backup.original()?.as_ref())?;
        }
        registry::remove_tree(&self.state)?;
        Ok(())
    }

    fn enable(&self, executable: &Path, proxy: &Path, label: &str) -> io::Result<()> {
        if !executable.is_absolute()
            || !proxy.is_absolute()
            || !executable.is_file()
            || !proxy.is_file()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "File-manager executables must exist at absolute paths",
            ));
        }
        let mut command = OsString::from("\"");
        command.push(proxy.as_os_str());
        command.push("\" --open \"%1\\.\"");
        let command = Value::text(&command)?;
        let mut watch = OsString::from("\"");
        watch.push(proxy.as_os_str());
        watch.push("\" --watch");
        let watch = Value::text(&watch)?;
        if let Some(owner) = self.owner()? {
            if same_path(&owner, executable) && self.command()?.as_ref() == Some(&command) {
                // Resume an interrupted registration without replacing its backup.
            } else {
                self.disable(&owner)?;
            }
        }
        if self.owner()?.is_none() {
            for class in CLASSES {
                if registry::get(&format!(r"{}\command", self.verb(class)), "")?.is_some() {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        "Another registration owns the Strata folder verb",
                    ));
                }
            }
            // Flush the complete rollback journal before overriding a shell default.
            for class in CLASSES {
                let original = registry::get(&self.shell(class), "")?;
                registry::set(&self.state, class, Some(&Value::backup(original)))?;
            }
            registry::set(&self.state, "Command", Some(&command))?;
            registry::set(&self.state, "Proxy", Some(&Value::text(proxy.as_os_str())?))?;
            registry::set(
                &self.state,
                "RunBackup",
                Some(&Value::backup(registry::get(&self.run, RUN_VALUE)?)),
            )?;
            registry::set(&self.state, "WatchCommand", Some(&watch))?;
            registry::set(
                &self.state,
                "Executable",
                Some(&Value::text(executable.as_os_str())?),
            )?;
            registry::flush(&self.state)?;
        }
        let result = (|| {
            registry::set(&self.run, RUN_VALUE, Some(&watch))?;
            for class in CLASSES {
                registry::set(
                    &format!(r"{}\command", self.verb(class)),
                    "",
                    Some(&command),
                )?;
                registry::set(
                    &self.verb(class),
                    "",
                    Some(&Value::text(OsStr::new(label))?),
                )?;
                registry::set(
                    &self.shell(class),
                    "",
                    Some(&Value::text(OsStr::new(VERB))?),
                )?;
            }
            Ok(())
        })();
        if result.is_err() {
            let _ = self.disable(executable);
        }
        result
    }
}

/// Return whether both per-user filesystem folder handlers belong to this app.
pub fn enabled(executable: &Path) -> io::Result<bool> {
    let _guard = lock::acquire()?;
    Registration::default().enabled(executable)
}

/// A per-user proxy outside the installation survives deletion of a portable folder.
pub fn proxy_path() -> io::Result<PathBuf> {
    let root = std::env::var_os("LOCALAPPDATA")
        .ok_or_else(|| io::Error::other("Missing local application-data directory"))?;
    let root = PathBuf::from(root);
    if !root.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Application-data directory must be absolute",
        ));
    }
    Ok(root
        .join("Strata")
        .join("ShellIntegration")
        .join("strata-shell.exe"))
}

/// Install the standalone proxy atomically, then register it under one per-user
/// lock so an uninstall or another installation cannot interleave its changes.
pub fn install(
    executable: &Path,
    source: &Path,
    label: &str,
    activation: Option<(&str, &[u8])>,
) -> io::Result<()> {
    let proxy = proxy_path()?;
    // An executable cannot be atomically replaced while its watcher is running.
    if std::fs::read(source).ok() != std::fs::read(&proxy).ok() {
        watcher::stop()?;
    }
    let _guard = lock::acquire()?;
    let parent = proxy.parent().expect("proxy directory");
    for path in [
        proxy.as_path(),
        parent,
        parent.parent().expect("application directory"),
    ] {
        match std::fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_attributes() & 0x400 != 0 => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Refusing a redirected shell-helper path",
                ));
            }
            Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
            _ => {}
        }
    }
    std::fs::create_dir_all(parent)?;
    if !source.is_file() || std::fs::metadata(source)?.len() > 64 * 1024 * 1024 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Standalone shell helper is missing or oversized",
        ));
    }
    let bytes = std::fs::read(source)?;
    let copied = std::fs::read(&proxy).ok().as_deref() != Some(bytes.as_slice());
    if copied {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_nanos();
        let temporary = parent.join(format!("proxy-{}-{nonce}.tmp", std::process::id()));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        let result = (|| {
            file.write_all(&bytes)?;
            file.sync_all()?;
            drop(file);
            std::fs::rename(&temporary, &proxy)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        result?;
    }
    let registration = Registration::default();
    let expected_label = Some(Value::text(OsStr::new(label))?);
    if !copied
        && registration.enabled(executable)?
        && CLASSES.into_iter().all(|class| {
            registry::get(&registration.verb(class), "").ok() == Some(expected_label.clone())
        })
    {
        registration.set_activation(activation)?;
        drop(_guard);
        return watcher::ensure(&proxy);
    }
    let result = registration
        .enable(executable, &proxy, label)
        .and_then(|()| registration.set_activation(activation));
    notify();
    drop(_guard);
    result?;
    if let Err(error) = watcher::ensure(&proxy) {
        let _ = disable(executable);
        return Err(error);
    }
    Ok(())
}

/// Register only filesystem folders and drives. Desktop, taskbar and virtual
/// namespaces retain their Windows handlers.
pub fn enable(executable: &Path, proxy: &Path, label: &str) -> io::Result<()> {
    let _guard = lock::acquire()?;
    let result = Registration::default().enable(executable, proxy, label);
    notify();
    result
}

/// Restore the exact prior defaults only if this installation still owns them.
pub fn disable(executable: &Path) -> io::Result<()> {
    let _guard = lock::acquire()?;
    let registration = Registration::default();
    let owned = registration
        .owner()?
        .is_some_and(|owner| same_path(&owner, executable));
    if !owned {
        return Ok(());
    }
    let proxy = registry::get(&registration.state, "Proxy")?
        .and_then(|value| value.os_string())
        .map(PathBuf::from);
    let result = registration.disable(executable);
    drop(_guard);
    if result.is_ok() {
        watcher::stop()?;
    }
    if result.is_ok()
        && owned
        && let (Some(proxy), Ok(expected)) = (proxy, proxy_path())
        && same_path(&proxy, &expected)
        && std::env::current_exe().is_ok_and(|current| !same_path(&current, &proxy))
    {
        // A currently executing proxy may still hold the file open; its
        // association is already restored, so a leftover cache is harmless.
        let _ = std::fs::remove_file(proxy);
    }
    notify();
    result
}

/// The stable proxy survives removal of a portable app directory. On its first
/// invocation after removal it repairs the association before opening Explorer.
pub fn open(folder: &OsStr) -> io::Result<()> {
    open_with_request(folder, None)
}

/// Route a native shell invocation straight to the owning installation's pipe.
pub fn open_with_request(folder: &OsStr, request: Option<&[u8]>) -> io::Result<()> {
    let registration = Registration::default();
    let target = (|| -> io::Result<Option<PathBuf>> {
        let _guard = lock::acquire()?;
        let before = registration.owner()?;
        let available = registration.available_target();
        if before.is_some() && available.as_ref().is_ok_and(Option::is_none) {
            notify();
        }
        available
    })();
    if let Err(error) = &target {
        eprintln!("Unable to inspect or repair folder handler: {error}");
    }
    if let Ok(Some(executable)) = target {
        if let Some(request) = request
            && registration.forward_running(Some(request))
        {
            return Ok(());
        }
        if let Ok(mut child) = Command::new(&executable).arg("--").arg(folder).spawn() {
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(150);
            loop {
                match child.try_wait() {
                    Err(_) => break,
                    Ok(Some(status)) if !status.success() => break,
                    Ok(Some(_)) => return Ok(()),
                    Ok(None) if std::time::Instant::now() >= deadline => return Ok(()),
                    Ok(None) => std::thread::sleep(std::time::Duration::from_millis(10)),
                }
            }
        }
        if let Err(error) = disable(&executable) {
            eprintln!("Unable to repair folder handler: {error}");
        }
    }
    let windows = std::env::var_os("SystemRoot")
        .ok_or_else(|| io::Error::other("Missing Windows directory"))?;
    Command::new(PathBuf::from(windows).join("explorer.exe"))
        .arg(folder)
        .spawn()?;
    Ok(())
}

#[cfg(test)]
mod tests;
