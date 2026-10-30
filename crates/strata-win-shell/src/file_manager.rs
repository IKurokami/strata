// SPDX-License-Identifier: MIT

use std::{
    ffi::OsStr,
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
mod journal;
mod launch;
mod routing;
mod status;
pub use status::{Actual, IntegrationStatus, RouteStatus};
mod monitor;
pub use monitor::{RegistryMonitor, watch_changes};

mod lock;
mod watcher;
pub use watcher::watch;

const VERB: &str = "Strata.Open";
const RUN_VALUE: &str = "StrataFileManager";
const CLASSES: [&str; 2] = ["Directory", "Drive"];

#[derive(Clone)]
struct Registration {
    classes: String,
    state: String,
    run: String,
    legacy_state: Option<String>,
}

impl Default for Registration {
    fn default() -> Self {
        Self {
            classes: r"Software\Classes".into(),
            state: r"Software\Strata\FileManagerV2".into(),
            run: r"Software\Microsoft\Windows\CurrentVersion\Run".into(),
            legacy_state: Some(r"Software\Strata\FileManager".into()),
        }
    }
}

pub(crate) fn same_path(first: &Path, second: &Path) -> bool {
    fn normalized(path: &Path) -> Vec<u16> {
        let value: Vec<u16> = path.as_os_str().encode_wide().collect();
        let prefix: Vec<u16> = r"\\?\".encode_utf16().collect();
        let unc: Vec<u16> = r"\\?\UNC\".encode_utf16().collect();
        if value.starts_with(&unc) {
            r"\\"
                .encode_utf16()
                .chain(value[unc.len()..].iter().copied())
                .collect()
        } else if value.starts_with(&prefix) {
            value[prefix.len()..].to_vec()
        } else {
            value
        }
    }
    let first = normalized(first);
    let second = normalized(second);
    // SAFETY: Both length-delimited UTF-16 buffers remain valid for the call.
    unsafe { CompareStringOrdinal(&first, &second, true) == CSTR_EQUAL }
}

fn notify() {
    // SAFETY: Association-change notification takes no item pointers.
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
}

/// Match GLib's Windows configuration root without loading GLib or GTK.
pub fn configuration_directory() -> io::Result<PathBuf> {
    if let Some(path) = std::env::var_os("XDG_CONFIG_HOME").filter(|path| !path.is_empty()) {
        return Ok(PathBuf::from(path).join("strata"));
    }
    use std::os::windows::ffi::OsStringExt;
    use windows::Win32::{
        System::Com::CoTaskMemFree,
        UI::Shell::{FOLDERID_LocalAppData, KF_FLAG_DEFAULT, SHGetKnownFolderPath},
    };
    // SAFETY: The current-user known-folder call returns one independently allocated string.
    let raw = unsafe { SHGetKnownFolderPath(&FOLDERID_LocalAppData, KF_FLAG_DEFAULT, None) }
        .map_err(io::Error::other)?;
    // SAFETY: A successful known-folder query returned a terminated UTF-16 allocation.
    let path = PathBuf::from(unsafe { std::ffi::OsString::from_wide(raw.as_wide()) });
    // SAFETY: Release the string allocated by SHGetKnownFolderPath exactly once.
    unsafe {
        CoTaskMemFree(Some(raw.0.cast()));
    }
    Ok(path.join("strata"))
}

impl Registration {
    fn metadata(&self, name: &str) -> io::Result<Option<Value>> {
        match registry::get(&self.state, name)? {
            Some(value) => Ok(Some(value)),
            None => self
                .legacy_state
                .as_ref()
                .map(|state| registry::get(state, name))
                .transpose()
                .map(Option::flatten),
        }
    }

    fn import_legacy(&self) -> io::Result<()> {
        if registry::get(&self.state, journal::NAME)?.is_some() {
            return Ok(());
        }
        let Some(legacy) = &self.legacy_state else {
            return Ok(());
        };
        if registry::get(legacy, "Executable")?.is_none() {
            return Ok(());
        }
        for name in [
            "Executable",
            "Proxy",
            "Command",
            "WatchCommand",
            "RunBackup",
            "Directory",
            "Drive",
            "Activation",
        ] {
            if let Some(value) = registry::get(legacy, name)? {
                registry::set(&self.state, name, Some(&value))?;
                registry::set(
                    &self.state,
                    &format!("Legacy.{name}"),
                    Some(&Value::backup(Some(value))),
                )?;
            }
        }
        registry::flush(&self.state)
    }

    fn retire_legacy(&self) -> io::Result<()> {
        let Some(legacy) = &self.legacy_state else {
            return Ok(());
        };
        for name in [
            "Executable",
            "Proxy",
            "Command",
            "WatchCommand",
            "RunBackup",
            "Directory",
            "Drive",
            "Activation",
        ] {
            if let Some(snapshot) = registry::get(&self.state, &format!("Legacy.{name}"))? {
                if registry::get(legacy, name)? == snapshot.original()? {
                    registry::set(legacy, name, None)?;
                }
                registry::set(&self.state, &format!("Legacy.{name}"), None)?;
            }
        }
        registry::flush(legacy)?;
        registry::delete_empty_key(legacy)
    }
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

    #[cfg(test)]
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
        let validate = |value: Value| {
            value
                .os_string()
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "Invalid integration owner")
                })
        };
        if let Some(journal) = journal::Journal::read(self)? {
            return validate(journal.executable).map(Some);
        }
        if let Some(legacy) = &self.legacy_state {
            if let Some(owner) = registry::get(legacy, "Executable")? {
                return validate(owner).map(Some);
            }
        }
        if let Some(owner) = self.metadata("Executable")? {
            return validate(owner).map(Some);
        }
        if self.metadata("Command")?.is_some() || self.metadata("Proxy")?.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Missing integration owner",
            ));
        }
        Ok(None)
    }

    fn command(&self) -> io::Result<Option<Value>> {
        self.metadata("Command")
    }

    fn available_target(&self) -> io::Result<Option<PathBuf>> {
        if let Some(mut journal) = journal::Journal::load_if_present(self)?
            && matches!(
                journal.phase,
                journal::Phase::Prepared | journal::Phase::Restoring
            )
        {
            journal.restore(self)?;
        }
        let Some(executable) = self.owner()? else {
            return Ok(None);
        };
        if std::fs::symlink_metadata(&executable)
            .is_ok_and(|m| m.is_file() && m.file_attributes() & 0x400 == 0)
        {
            return Ok(Some(executable));
        }
        self.disable(&executable)?;
        Ok(None)
    }
}

/// Return whether both per-user filesystem folder handlers belong to this app.
pub fn enabled(executable: &Path) -> io::Result<bool> {
    Ok(probe_status(Some(executable))?.actual == Actual::Active)
}

/// Installer upgrade only reconciles an existing registration owned by this target.
pub fn upgrade_owned(executable: &Path, source: &Path) -> io::Result<()> {
    if !executable.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Expected an absolute integration target",
        ));
    }
    let (label, activation) = {
        let _guard = lock::acquire()?;
        let registration = Registration::default();
        if !registration
            .owner()?
            .is_some_and(|owner| same_path(&owner, executable))
        {
            return Ok(());
        }
        let label = registration
            .metadata("Label")?
            .or(registry::get(&registration.verb("Directory"), "")?)
            .and_then(|v| v.os_string())
            .and_then(|v| v.into_string().ok())
            .ok_or_else(|| io::Error::other("Missing integration label"))?;
        let activation = registration.activation_data()?;
        (label, activation)
    };
    install_with_policy(
        executable,
        source,
        &label,
        activation
            .as_ref()
            .map(|(scope, request)| (scope.as_str(), request.as_slice())),
        false,
    )
}

impl Registration {
    fn activation_data(&self) -> io::Result<Option<(String, Vec<u8>)>> {
        let Some(value) = self.metadata("Activation")? else {
            return Ok(None);
        };
        if value.kind != windows::Win32::System::Registry::REG_BINARY || value.data.len() < 4 {
            return Ok(None);
        }
        let length =
            u32::from_le_bytes(value.data[..4].try_into().expect("activation header")) as usize;
        if length == 0 || length > 32 * 1024 || length >= value.data.len() - 4 {
            return Ok(None);
        }
        Ok(std::str::from_utf8(&value.data[4..4 + length])
            .ok()
            .map(|scope| (scope.to_owned(), value.data[4 + length..].to_vec())))
    }
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
        .join("strata-shell-v2.exe"))
}

/// Install the standalone proxy atomically, then register it under one per-user
/// lock so an uninstall or another installation cannot interleave its changes.
pub fn install(
    executable: &Path,
    source: &Path,
    label: &str,
    activation: Option<(&str, &[u8])>,
) -> io::Result<()> {
    install_with_policy(executable, source, label, activation, false)
}

/// `replace_foreign` is allowed only after an explicit user confirmation.
pub fn install_with_policy(
    executable: &Path,
    source: &Path,
    label: &str,
    activation: Option<(&str, &[u8])>,
    replace_foreign: bool,
) -> io::Result<()> {
    let proxy = proxy_path()?;
    let guard = lock::acquire()?;
    let registration = Registration::default();
    if !replace_foreign
        && registration
            .owner()?
            .is_some_and(|owner| !same_path(&owner, executable))
    {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "Another installation owns the file-manager registration",
        ));
    }
    let parent = proxy.parent().expect("proxy parent");
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
    if !source.is_absolute()
        || !executable.is_absolute()
        || !source.is_file()
        || std::fs::metadata(source)?.len() > 64 * 1024 * 1024
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Standalone shell helper is missing or oversized",
        ));
    }
    if std::fs::symlink_metadata(source)?.file_attributes() & 0x400 != 0
        || std::fs::symlink_metadata(executable)?.file_attributes() & 0x400 != 0
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Refusing a redirected integration executable",
        ));
    }
    {
        use std::{os::windows::process::CommandExt, process::Stdio};
        let protocol = Command::new(source)
            .arg("--integration-protocol")
            .creation_flags(0x08000000)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()?;
        if !protocol.status.success()
            || protocol.stdout != b"strata-shell-registry-v2\n"
                && protocol.stdout != b"strata-shell-registry-v2\r\n"
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Standalone helper does not support Registry routing v2",
            ));
        }
    }
    std::fs::create_dir_all(parent)?;
    let bytes = std::fs::read(source)?;
    if std::fs::read(&proxy).ok().as_deref() != Some(bytes.as_slice()) {
        let temporary = parent.join(format!(
            "proxy-{}-{}.tmp",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(io::Error::other)?
                .as_nanos()
        ));
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
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
    registration.enable_policy(executable, &proxy, label, replace_foreign)?;
    registration.set_activation(activation)?;
    notify();
    drop(guard);
    // A versioned proxy keeps the old watcher usable until new routes are verified.
    watcher::stop()?;
    let _guard = lock::acquire()?;
    registration.retire_legacy()?;
    Ok(())
}

/// Registry verification is distinct from interactive Win+E compatibility evidence.
pub fn probe_status(executable: Option<&Path>) -> io::Result<IntegrationStatus> {
    let guard = lock::acquire()?;
    let mut status = Registration::default().probe(executable)?;
    drop(guard);
    status.legacy_watcher_running = watcher::running();
    if status.actual == Actual::Active && status.legacy_watcher_running {
        status.actual = Actual::Partial;
    }
    Ok(status)
}

/// Register the folder, drive and Win+E Shell routes. Virtual namespaces are
/// outside the native-path dispatch boundary and need interactive acceptance.
pub fn enable(executable: &Path, proxy: &Path, label: &str) -> io::Result<()> {
    let _guard = lock::acquire()?;
    let result = Registration::default().enable(executable, proxy, label);
    notify();
    result
}

/// Restore the exact prior defaults only if this installation still owns them.
pub fn disable(executable: &Path) -> io::Result<()> {
    if !executable.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Expected an absolute integration target",
        ));
    }
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
    if !Path::new(folder).is_absolute() || folder.encode_wide().any(|unit| unit == 0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Only absolute native folders are supported",
        ));
    }
    let native_request;
    let request = if let Some(request) = request {
        request
    } else {
        let cwd = std::env::current_dir()?
            .as_os_str()
            .encode_wide()
            .collect::<Vec<_>>();
        let arguments = folder.encode_wide().collect::<Vec<_>>();
        native_request =
            format!("{{\"Open\":{{\"cwd\":{cwd:?},\"arguments\":[{arguments:?}]}}}}").into_bytes();
        native_request.as_slice()
    };
    dispatch(Some(folder), Some(request))
}

/// Win+E activates the primary without inventing a folder argument.
pub fn activate() -> io::Result<()> {
    dispatch(None, None)
}

fn dispatch(folder: Option<&OsStr>, request: Option<&[u8]>) -> io::Result<()> {
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
    let target = target?;
    if let Some(executable) = target {
        let result = registration.launch_once(request, || {
            let mut launch = Command::new(&executable);
            if let Some(folder) = folder {
                launch.arg("--").arg(folder);
            }
            launch
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            Ok(launch.spawn()?.id())
        });
        match result {
            Ok(()) => return Ok(()),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::TimedOut
                        | io::ErrorKind::WouldBlock
                        | io::ErrorKind::ConnectionAborted
                ) =>
            {
                return Err(error);
            }
            Err(_) => {}
        }
        disable(&executable)?;
    }
    // Never invoke Explorer while one of our route commands could call us again.
    if registration.owns_any_route()? {
        return Err(io::Error::other(
            "Shell fallback requires successful rollback",
        ));
    }
    if std::env::var_os("STRATA_SHELL_FALLBACK_DEPTH").is_some() {
        return Err(io::Error::other("Recursive shell fallback refused"));
    }
    let windows = std::env::var_os("SystemRoot")
        .ok_or_else(|| io::Error::other("Missing Windows directory"))?;
    let mut fallback = Command::new(PathBuf::from(windows).join("explorer.exe"));
    fallback.env("STRATA_SHELL_FALLBACK_DEPTH", "1");
    if let Some(folder) = folder {
        fallback.arg(folder);
    }
    fallback.spawn()?;
    Ok(())
}

#[cfg(test)]
mod tests;
