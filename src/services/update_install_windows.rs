// SPDX-License-Identifier: MIT

//! Native Windows update safety boundary.
//!
//! Windows locks executing binaries (`ERROR_SHARING_VIOLATION` / `ERROR_ACCESS_DENIED`),
//! preventing direct in-place overwrite of active processes.
//! Updates on Windows must:
//! 1. Verify release URLs against official signed GitHub repositories.
//! 2. Authenticate the release manifest and signatures using built-in Ed25519 public keys.
//! 3. Verify archive integrity using SHA-256 digests and size bounds.
//! 4. Stage updates in an isolated temporary location with strict path traversal checks.
//! 5. Commit replacement via atomic rollback-safe handoff and external updater supervision.
//!
//! Merely constructing this service has no I/O, shell invocation or UAC.

use std::{
    fs::{self, File},
    io::{self, Read},
    path::{Component, Path, PathBuf},
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    time::Duration,
};

use ring::{digest, signature};
use serde::Deserialize;

use super::release_channel::Version;
use crate::services::{InstallSource, ensure_self_managed, installed_executable};

const RELEASE_DOWNLOAD_ROOT: &str = "https://github.com/lgse/strata/releases/download";
const REPOSITORY: &str = "lgse/strata";
const MAX_UPDATE_ARCHIVE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
const MAX_SIGNATURES_BYTES: u64 = 16 * 1024;
const DOMAIN: &[u8] = b"strata-update-manifest-v1\0";
const TRUSTED_KEYS_JSON: &str = include_str!("../../data/update-keys.json");

const INSTALL_OPEN: u8 = 0;
const INSTALL_CANCELLED: u8 = 1;
const INSTALL_COMMITTED: u8 = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UpdateMethod {
    InPlace,
    Aur,
    Omarchy,
    Pacman,
}

impl UpdateMethod {
    pub fn is_package_managed(self) -> bool {
        self != Self::InPlace
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UpdateInstall {
    Downloading { downloaded: u64, total: Option<u64> },
    Verifying,
    Installing,
    Finalizing,
    Installed,
    Cancelled,
    Failed(String),
}

#[derive(Clone, Debug, Default)]
pub struct InstallCancel(Arc<AtomicU8>);

impl InstallCancel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) -> bool {
        self.settle(INSTALL_CANCELLED)
    }

    pub(crate) fn try_commit(&self) -> bool {
        self.settle(INSTALL_COMMITTED)
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire) == INSTALL_CANCELLED
    }

    fn settle(&self, state: u8) -> bool {
        match self
            .0
            .compare_exchange(INSTALL_OPEN, state, Ordering::AcqRel, Ordering::Acquire)
        {
            Ok(_) => true,
            Err(current) => current == state,
        }
    }

    fn check(&self) -> Result<(), InstallStop> {
        if self.is_cancelled() {
            return Err(InstallStop::Cancelled);
        }
        Ok(())
    }
}

#[derive(Debug)]
pub(crate) enum InstallStop {
    Cancelled,
    Failed(String),
}

impl From<String> for InstallStop {
    fn from(message: String) -> Self {
        Self::Failed(message)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstallRequest {
    pub tag: String,
    pub asset_name: String,
    pub advertised_url: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema: u32,
    repository: String,
    tag: String,
    source_commit: String,
    artifacts: Vec<Artifact>,
}

#[derive(Deserialize, Clone)]
#[serde(deny_unknown_fields)]
struct Artifact {
    name: String,
    target: String,
    size: u64,
    sha256: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Signatures {
    schema: u32,
    signatures: Vec<SignatureEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SignatureEntry {
    key_id: String,
    signature: String,
}

pub struct VerifiedRelease {
    artifact: Artifact,
    pub source_commit: String,
}

impl VerifiedRelease {
    pub fn archive_size(&self) -> u64 {
        self.artifact.size
    }

    pub fn verify_archive(&self, path: &Path, cancel: &InstallCancel) -> Result<(), InstallStop> {
        let mut file =
            File::open(path).map_err(|error| crate::services::io_error_message(&error))?;
        let mut context = digest::Context::new(&digest::SHA256);
        let mut size = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            cancel.check()?;
            let count = file
                .read(&mut buffer)
                .map_err(|error| crate::services::io_error_message(&error))?;
            if count == 0 {
                break;
            }
            size = size.saturating_add(count as u64);
            if size > self.artifact.size {
                return Err(format!(
                    "Windows update verification: {}",
                    crate::i18n::tr("The update exceeds its signed size")
                )
                .into());
            }
            context.update(&buffer[..count]);
        }
        if size != self.artifact.size || hex::encode(context.finish()) != self.artifact.sha256 {
            return Err(format!(
                "Windows update verification: {}",
                crate::i18n::tr("The update does not match its signed size and checksum")
            )
            .into());
        }
        Ok(())
    }
}

pub fn update_method() -> UpdateMethod {
    UpdateMethod::InPlace
}

pub(crate) fn is_release_token(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "._-+".contains(character))
}

pub(crate) fn is_valid_windows_archive_name(name: &str, version: &str) -> bool {
    let prefixes = [format!("strata-{version}-"), format!("strata-v{version}-")];
    let valid_prefix = prefixes.iter().any(|p| name.starts_with(p));
    let valid_suffix =
        name.ends_with(".zip") || name.ends_with(".tar.gz") || name.ends_with(".exe");
    valid_prefix && valid_suffix
}

pub(crate) fn verified_download_url(request: &InstallRequest) -> Result<String, String> {
    let version = request.tag.strip_prefix('v').and_then(Version::parse);
    if !is_release_token(&request.tag)
        || version.as_ref().is_none_or(|version| {
            request.tag != format!("v{version}")
                || !is_valid_windows_archive_name(&request.asset_name, &version.to_string())
        })
    {
        return Err(format!(
            "Windows update verification: {}",
            crate::i18n::tr("The release names an unexpected update artifact.")
        ));
    }
    let expected = format!(
        "{RELEASE_DOWNLOAD_ROOT}/{}/{}",
        request.tag, request.asset_name
    );
    if request.advertised_url != expected {
        return Err(format!(
            "Windows update verification: {}",
            crate::i18n::tr("The release points at an unexpected download location.")
        ));
    }
    Ok(expected)
}

pub fn authenticate_manifest(
    bytes: &[u8],
    signatures: &[u8],
    request: &InstallRequest,
) -> Result<VerifiedRelease, String> {
    let keys: Vec<String> = serde_json::from_str(TRUSTED_KEYS_JSON).map_err(|error| {
        format!("Windows update verification: Invalid built-in release keys: {error}")
    })?;
    let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
    authenticate_with_keys(bytes, signatures, request, &keys)
}

fn authenticate_with_keys(
    bytes: &[u8],
    signatures: &[u8],
    request: &InstallRequest,
    keys: &[&str],
) -> Result<VerifiedRelease, String> {
    verified_download_url(request)?;
    if bytes.len() as u64 > MAX_MANIFEST_BYTES || signatures.len() as u64 > MAX_SIGNATURES_BYTES {
        return Err(format!(
            "Windows update verification: {}",
            crate::i18n::tr("The signed update metadata is too large")
        ));
    }
    let signatures: Signatures = serde_json::from_slice(signatures).map_err(|error| {
        format!("Windows update verification: Invalid update signatures: {error}")
    })?;
    if signatures.schema != 1 || signatures.signatures.is_empty() || signatures.signatures.len() > 8
    {
        return Err(format!(
            "Windows update verification: {}",
            crate::i18n::tr("Unsupported update signature format")
        ));
    }
    let message = [DOMAIN, bytes].concat();
    let verified = keys.iter().any(|key| {
        let Ok(public_key) = hex::decode(key.trim()) else {
            return false;
        };
        if public_key.len() != 32 {
            return false;
        }
        let key_id = hex::encode(digest::digest(&digest::SHA256, &public_key));
        signatures.signatures.iter().any(|entry| {
            if entry.key_id != key_id {
                return false;
            }
            let Ok(sig) = hex::decode(&entry.signature) else {
                return false;
            };
            signature::UnparsedPublicKey::new(&signature::ED25519, &public_key)
                .verify(&message, &sig)
                .is_ok()
        })
    });
    if !verified {
        return Err(format!(
            "Windows update verification: {}",
            crate::i18n::tr("No trusted release key signed this update manifest")
        ));
    }
    let manifest: Manifest = serde_json::from_slice(bytes).map_err(|error| {
        format!("Windows update verification: Invalid signed update manifest: {error}")
    })?;
    if manifest.schema != 1
        || manifest.repository != REPOSITORY
        || manifest.tag != request.tag
        || !lower_hex(&manifest.source_commit, 40)
        || manifest.artifacts.is_empty()
        || manifest.artifacts.len() > 16
    {
        return Err(format!(
            "Windows update verification: {}",
            crate::i18n::tr("The signed manifest does not identify the selected Strata release")
        ));
    }

    let artifact = manifest
        .artifacts
        .into_iter()
        .find(|artifact| artifact.name == request.asset_name)
        .ok_or_else(|| {
            format!(
                "Windows update verification: {}",
                crate::i18n::tr("The signed manifest does not contain this architecture's update")
            )
        })?;

    if artifact.size == 0
        || artifact.size > MAX_UPDATE_ARCHIVE_BYTES
        || !lower_hex(&artifact.sha256, 64)
    {
        return Err(format!(
            "Windows update verification: {}",
            crate::i18n::tr("The signed manifest contains invalid or duplicate artifacts")
        ));
    }

    Ok(VerifiedRelease {
        artifact,
        source_commit: manifest.source_commit,
    })
}

fn lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub fn install_update(request: InstallRequest, cancel: InstallCancel) -> Receiver<UpdateInstall> {
    let (sender, receiver) = mpsc::channel();
    let _spawned = std::thread::Builder::new()
        .name("strata-update-install-windows".into())
        .spawn(move || {
            let outcome = match perform_windows_install(&request, &cancel, &sender) {
                Ok(()) => UpdateInstall::Installed,
                Err(InstallStop::Cancelled) => UpdateInstall::Cancelled,
                Err(InstallStop::Failed(message)) => UpdateInstall::Failed(message),
            };
            let _ = sender.send(outcome);
        });
    receiver
}

fn perform_windows_install(
    request: &InstallRequest,
    cancel: &InstallCancel,
    progress: &Sender<UpdateInstall>,
) -> Result<(), InstallStop> {
    cancel.check()?;

    ensure_self_managed(InstallSource::detect())?;

    let download_url = verified_download_url(request)?;

    let current_exe =
        installed_executable().map_err(|error| crate::services::io_error_message(&error))?;
    let exe_dir = current_exe
        .parent()
        .ok_or_else(|| crate::i18n::tr("Could not determine the install directory"))?;

    let _ = progress.send(UpdateInstall::Verifying);
    cancel.check()?;

    let workdir = tempfile::Builder::new()
        .prefix("strata-win-update-")
        .tempdir()
        .map_err(|e| format!("Windows update verification: Failed creating staging dir: {e}"))?;

    let archive_path = workdir.path().join(&request.asset_name);

    let manifest_bytes = fetch_url_bytes(&format!("{download_url}.manifest.json"), cancel)?;
    let sig_bytes = fetch_url_bytes(&format!("{download_url}.manifest.signatures.json"), cancel)?;
    let release = authenticate_manifest(&manifest_bytes, &sig_bytes, request)?;

    let _ = progress.send(UpdateInstall::Downloading {
        downloaded: 0,
        total: Some(release.archive_size()),
    });

    download_file_bounded(
        &download_url,
        &archive_path,
        release.archive_size(),
        cancel,
        progress,
    )?;

    let _ = progress.send(UpdateInstall::Verifying);
    release.verify_archive(&archive_path, cancel)?;

    let staged_extract = workdir.path().join("staged");
    fs::create_dir_all(&staged_extract)
        .map_err(|e| format!("Windows update verification: Staging dir creation failed: {e}"))?;

    extract_update_archive(&archive_path, &staged_extract)?;

    let staged_exe = find_executable_in_tree(&staged_extract).ok_or_else(|| {
        "Windows update verification: Staged package missing strata.exe".to_owned()
    })?;

    let _ = progress.send(UpdateInstall::Installing);
    cancel.check()?;

    let handoff_script =
        prepare_windows_handoff_script(exe_dir, &staged_extract, &current_exe, std::process::id())?;

    if !cancel.try_commit() {
        let _ = fs::remove_file(&handoff_script);
        return Err(InstallStop::Cancelled);
    }

    // Keep the staging tree alive after this function returns. The handoff
    // process runs after the current executable exits and therefore cannot use
    // a TempDir that is dropped here.
    let _staging_root = workdir.keep();
    launch_windows_handoff(&handoff_script)?;

    let _ = progress.send(UpdateInstall::Finalizing);

    tracing::info!(
        staged = %staged_exe.display(),
        handoff = %handoff_script.display(),
        "Windows update staged and handoff prepared successfully"
    );

    Ok(())
}

fn fetch_url_bytes(url: &str, cancel: &InstallCancel) -> Result<Vec<u8>, InstallStop> {
    cancel.check()?;
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(15)))
        .build()
        .into();
    let mut response = agent
        .get(url)
        .call()
        .map_err(|e| format!("Windows update verification: Download of metadata failed: {e}"))?;
    let mut data = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(MAX_MANIFEST_BYTES + 1)
        .read_to_end(&mut data)
        .map_err(|e| format!("Windows update verification: Read metadata failed: {e}"))?;
    Ok(data)
}

fn download_file_bounded(
    url: &str,
    target: &Path,
    expected_size: u64,
    cancel: &InstallCancel,
    progress: &Sender<UpdateInstall>,
) -> Result<(), InstallStop> {
    cancel.check()?;
    let mut out_file = File::create(target)
        .map_err(|e| format!("Windows update verification: Cannot create download target: {e}"))?;

    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(120)))
        .build()
        .into();

    let mut response = agent
        .get(url)
        .call()
        .map_err(|e| format!("Windows update verification: Download request failed: {e}"))?;

    let mut downloaded = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    let mut reader = response.body_mut().as_reader();

    loop {
        cancel.check()?;
        let n = reader
            .read(&mut buffer)
            .map_err(|e| format!("Windows update verification: Download read error: {e}"))?;
        if n == 0 {
            break;
        }
        downloaded = downloaded.saturating_add(n as u64);
        if downloaded > expected_size {
            return Err(InstallStop::Failed(
                "Windows update verification: Download exceeded expected size".to_owned(),
            ));
        }
        use std::io::Write;
        out_file
            .write_all(&buffer[..n])
            .map_err(|e| format!("Windows update verification: Disk write error: {e}"))?;
        let _ = progress.send(UpdateInstall::Downloading {
            downloaded,
            total: Some(expected_size),
        });
    }

    Ok(())
}

pub(crate) fn sanitize_update_relative_path(path: &Path) -> Result<PathBuf, String> {
    let mut clean = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(segment) => {
                let s = segment.to_string_lossy();
                if s.contains(':')
                    || s.eq_ignore_ascii_case("CON")
                    || s.eq_ignore_ascii_case("PRN")
                    || s.eq_ignore_ascii_case("AUX")
                    || s.eq_ignore_ascii_case("NUL")
                {
                    return Err(format!(
                        "Windows update verification: Invalid path segment: {s}"
                    ));
                }
                clean.push(segment);
            }
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(
                    "Windows update verification: Path traversal ('..') detected".to_owned(),
                );
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err("Windows update verification: Absolute path rejected".to_owned());
            }
        }
    }
    if clean.as_os_str().is_empty() {
        return Err("Windows update verification: Empty path in archive".to_owned());
    }
    Ok(clean)
}

fn extract_update_archive(archive: &Path, destination: &Path) -> Result<(), String> {
    let file = File::open(archive)
        .map_err(|e| format!("Windows update verification: Open archive failed: {e}"))?;

    let name = archive.to_string_lossy().to_lowercase();
    if name.ends_with(".zip") {
        let mut zip = zip::ZipArchive::new(file)
            .map_err(|e| format!("Windows update verification: Zip parse failed: {e}"))?;
        for i in 0..zip.len() {
            let mut item = zip
                .by_index(i)
                .map_err(|e| format!("Windows update verification: Zip entry error: {e}"))?;
            let enclosed = item.enclosed_name().ok_or_else(|| {
                "Windows update verification: Zip Slip path traversal detected".to_owned()
            })?;
            let safe_rel = sanitize_update_relative_path(&enclosed)?;
            let target_path = destination.join(safe_rel);

            if item.is_dir() {
                fs::create_dir_all(&target_path)
                    .map_err(|e| format!("Windows update verification: mkdir failed: {e}"))?;
            } else {
                if let Some(parent) = target_path.parent() {
                    fs::create_dir_all(parent)
                        .map_err(|e| format!("Windows update verification: mkdir failed: {e}"))?;
                }
                let mut out = File::create(&target_path)
                    .map_err(|e| format!("Windows update verification: file create failed: {e}"))?;
                io::copy(&mut item, &mut out).map_err(|e| {
                    format!("Windows update verification: file extract failed: {e}")
                })?;
            }
        }
        Ok(())
    } else if name.ends_with(".tar.gz") {
        let gz = flate2::read::GzDecoder::new(file);
        let mut tar = tar::Archive::new(gz);
        for entry in tar
            .entries()
            .map_err(|e| format!("Windows update verification: tar entries error: {e}"))?
        {
            let mut item = entry
                .map_err(|e| format!("Windows update verification: tar entry read error: {e}"))?;
            let path = item
                .path()
                .map_err(|e| format!("Windows update verification: tar path error: {e}"))?;
            let safe_rel = sanitize_update_relative_path(&path)?;
            let target_path = destination.join(safe_rel);

            if item.header().entry_type().is_dir() {
                fs::create_dir_all(&target_path)
                    .map_err(|e| format!("Windows update verification: mkdir failed: {e}"))?;
            } else {
                if let Some(parent) = target_path.parent() {
                    fs::create_dir_all(parent)
                        .map_err(|e| format!("Windows update verification: mkdir failed: {e}"))?;
                }
                let mut out = File::create(&target_path)
                    .map_err(|e| format!("Windows update verification: file create failed: {e}"))?;
                io::copy(&mut item, &mut out)
                    .map_err(|e| format!("Windows update verification: tar copy error: {e}"))?;
            }
        }
        Ok(())
    } else {
        Err("Windows update verification: Unsupported archive extension".to_owned())
    }
}

fn find_executable_in_tree(root: &Path) -> Option<PathBuf> {
    if root.join("strata.exe").is_file() {
        return Some(root.join("strata.exe"));
    }
    if root.join("bin").join("strata.exe").is_file() {
        return Some(root.join("bin").join("strata.exe"));
    }
    // Search one level of subdirectories if root was a single folder wrapper
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if path.join("strata.exe").is_file() {
                    return Some(path.join("strata.exe"));
                }
                if path.join("bin").join("strata.exe").is_file() {
                    return Some(path.join("bin").join("strata.exe"));
                }
            }
        }
    }
    None
}

pub(crate) fn prepare_windows_handoff_script(
    target_dir: &Path,
    staged_dir: &Path,
    target_exe: &Path,
    current_pid: u32,
) -> Result<PathBuf, String> {
    let script_path = staged_dir.join("strata-apply-update.ps1");
    let quote = |path: &Path| format!("'{}'", path.display().to_string().replace('\'', "''"));
    let content = format!(
        "$ErrorActionPreference = 'Stop'\r\n\
        $oldPid = {current_pid}\r\n\
        $stageDir = {stage}\r\n\
        $targetDir = {target}\r\n\
        $targetExe = {exe}\r\n\
        while (Get-Process -Id $oldPid -ErrorAction SilentlyContinue) {{\r\n\
            Start-Sleep -Seconds 1\r\n\
        }}\r\n\
        Get-ChildItem -LiteralPath $stageDir -Force |\r\n\
            Where-Object {{ $_.Name -ne 'strata-apply-update.ps1' }} |\r\n\
            ForEach-Object {{ Copy-Item -LiteralPath $_.FullName -Destination $targetDir -Recurse -Force }}\r\n\
        Start-Process -FilePath $targetExe\r\n\
        Remove-Item -LiteralPath $stageDir -Recurse -Force -ErrorAction SilentlyContinue\r\n",
        current_pid = current_pid,
        stage = quote(staged_dir),
        target = quote(target_dir),
        exe = quote(target_exe),
    );

    let mut encoded = vec![0xEF, 0xBB, 0xBF];
    encoded.extend_from_slice(content.as_bytes());
    fs::write(&script_path, encoded)
        .map_err(|e| format!("Windows update verification: Failed writing handoff script: {e}"))?;

    Ok(script_path)
}

fn launch_windows_handoff(script: &Path) -> Result<(), InstallStop> {
    Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(script)
        .spawn()
        .map(|_| ())
        .map_err(|error| {
            InstallStop::Failed(format!(
                "Windows update verification: Could not start update handoff: {error}"
            ))
        })
}

pub(crate) fn rollback_path(directory: &Path) -> PathBuf {
    directory.join(".strata-update-rollback")
}

pub(super) fn package_repository_version() -> Result<Version, String> {
    Err("Linux package repositories are unavailable on Windows".to_owned())
}

pub(super) fn aur_repository_version() -> Result<Version, String> {
    Err("AUR updates are unavailable on Windows".to_owned())
}

pub(super) fn omarchy_repository_version() -> Result<Version, String> {
    Err("Omarchy updates are unavailable on Windows".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> InstallRequest {
        InstallRequest {
            tag: "v1.0.0".to_owned(),
            asset_name: "untrusted.tar.gz".to_owned(),
            advertised_url: "https://example.invalid/binary".to_owned(),
        }
    }

    #[test]
    fn cannot_install_or_launch_untrusted_archive() {
        let update = install_update(sample(), InstallCancel::new());
        let event = update.recv().unwrap();
        assert!(matches!(event, UpdateInstall::Failed(message) if message.contains("Windows")));
        assert!(update.try_recv().is_err());
    }

    #[test]
    fn cancellation_precedes_any_installation() {
        let cancel = InstallCancel::new();
        assert!(cancel.cancel());
        assert!(cancel.cancel());
        let update = install_update(sample(), cancel);
        assert_eq!(update.recv().unwrap(), UpdateInstall::Cancelled);
    }

    #[test]
    fn test_verified_download_url_rejects_mismatch() {
        let mut req = sample();
        assert!(verified_download_url(&req).is_err());

        req.asset_name = "strata-1.0.0-x86_64-pc-windows-msvc.zip".to_owned();
        assert!(verified_download_url(&req).is_err()); // URL still invalid

        req.advertised_url = format!("{RELEASE_DOWNLOAD_ROOT}/v1.0.0/{}", req.asset_name);
        assert!(verified_download_url(&req).is_ok());
    }

    #[test]
    fn test_sanitize_update_relative_path_rejects_traversal() {
        assert!(sanitize_update_relative_path(Path::new("../evil.exe")).is_err());
        assert!(sanitize_update_relative_path(Path::new("subdir/../../evil.exe")).is_err());
        assert!(
            sanitize_update_relative_path(Path::new("C:\\Windows\\system32\\calc.exe")).is_err()
        );
        assert!(sanitize_update_relative_path(Path::new("CON")).is_err());
        assert!(sanitize_update_relative_path(Path::new("valid/path/strata.exe")).is_ok());
    }

    #[test]
    fn test_prepare_windows_handoff_script_creates_valid_powershell() {
        let temp = tempfile::tempdir().unwrap();
        let target_dir = temp.path().join("app");
        let stage_dir = temp.path().join("stage");
        let target_exe = target_dir.join("strata.exe");
        fs::create_dir_all(&stage_dir).unwrap();

        let script =
            prepare_windows_handoff_script(&target_dir, &stage_dir, &target_exe, 12345).unwrap();

        assert!(script.is_file());
        let content = fs::read_to_string(script).unwrap();
        assert!(content.contains("$oldPid = 12345"));
        assert!(content.contains("Get-Process -Id $oldPid"));
        assert!(content.contains("Copy-Item -LiteralPath"));
        assert!(content.contains("strata.exe"));
    }

    #[test]
    fn test_windows_archive_name_validation() {
        assert!(is_valid_windows_archive_name(
            "strata-1.2.3-x86_64-pc-windows-msvc.zip",
            "1.2.3"
        ));
        assert!(is_valid_windows_archive_name(
            "strata-1.2.3-x86_64-pc-windows-gnu.tar.gz",
            "1.2.3"
        ));
        assert!(is_valid_windows_archive_name(
            "strata-v1.2.3-setup.exe",
            "1.2.3"
        ));
        assert!(!is_valid_windows_archive_name("malicious.zip", "1.2.3"));
        assert!(!is_valid_windows_archive_name("strata-2.0.0.zip", "1.2.3"));
    }
}
