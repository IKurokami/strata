// SPDX-License-Identifier: MIT

use super::*;

fn sample() -> InstallRequest {
    InstallRequest {
        tag: "v1.0.0".to_owned(),
        asset_name: "untrusted.tar.gz".to_owned(),
        advertised_url: "https://example.invalid/binary".to_owned(),
    }
}

#[test]
fn portable_updates_replace_the_bundle_root_even_with_an_archive_wrapper() {
    let root = tempfile::tempdir().unwrap();
    let installed = root.path().join("installed/bin/strata.exe");
    let extracted = root.path().join("extract");
    let staged = extracted.join("Strata/bin/strata.exe");
    fs::create_dir_all(staged.parent().unwrap()).unwrap();
    fs::write(&staged, b"fixture").unwrap();
    let discovered = find_executable_in_tree(&extracted).unwrap();
    assert_eq!(discovered, staged);
    assert_eq!(
        bundle_root(&installed).unwrap(),
        root.path().join("installed")
    );
    assert_eq!(bundle_root(&discovered).unwrap(), extracted.join("Strata"));
    assert_eq!(
        bundle_root(&root.path().join("strata.exe")).unwrap(),
        root.path()
    );
}

#[test]
fn signed_windows_updates_bind_the_fork_architecture_and_archive() {
    use ring::signature::KeyPair;
    let document =
        signature::Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
    let key = signature::Ed25519KeyPair::from_pkcs8(document.as_ref()).unwrap();
    let public = hex::encode(key.public_key().as_ref());
    let name = super::super::update_check::archive_name("1.0.0");
    let request = InstallRequest {
        tag: "v1.0.0".into(),
        asset_name: name.clone(),
        advertised_url: format!("{RELEASE_DOWNLOAD_ROOT}/v1.0.0/{name}"),
    };
    let manifest = serde_json::json!({
        "schema": 1, "repository": REPOSITORY, "tag": "v1.0.0",
        "source_commit": "a".repeat(40),
        "artifacts": [{"name": name, "target": format!("{}-pc-windows-gnu", std::env::consts::ARCH),
            "size": 10, "sha256": "b".repeat(64)}],
    });
    let authenticate = |value: &serde_json::Value| {
        let bytes = serde_json::to_vec(value).unwrap();
        let signed = key.sign(&[DOMAIN, bytes.as_slice()].concat());
        let signatures = serde_json::to_vec(&serde_json::json!({"schema": 1, "signatures": [{
            "key_id": hex::encode(digest::digest(&digest::SHA256, key.public_key().as_ref())),
            "signature": hex::encode(signed.as_ref()),
        }]}))
        .unwrap();
        authenticate_with_keys(&bytes, &signatures, &request, &[&public])
    };
    assert!(authenticate(&manifest).is_ok());
    let mut wrong_repo = manifest.clone();
    wrong_repo["repository"] = "lgse/strata".into();
    assert!(authenticate(&wrong_repo).is_err());
    let mut wrong_target = manifest.clone();
    wrong_target["artifacts"][0]["target"] = "x86_64-unknown-linux-gnu".into();
    assert!(authenticate(&wrong_target).is_err());
    let mut wrong_archive = manifest;
    wrong_archive["artifacts"][0]["name"] = "strata-2.0.0-x86_64-pc-windows-gnu.zip".into();
    assert!(authenticate(&wrong_archive).is_err());
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

    req.asset_name = super::super::update_check::archive_name("1.0.0");
    assert!(verified_download_url(&req).is_err()); // URL still invalid

    req.advertised_url = format!("{RELEASE_DOWNLOAD_ROOT}/v1.0.0/{}", req.asset_name);
    assert!(verified_download_url(&req).is_ok());
    req.advertised_url = format!(
        "https://github.com/lgse/strata/releases/download/v1.0.0/{}",
        req.asset_name
    );
    assert!(verified_download_url(&req).is_err());
}

#[test]
fn test_sanitize_update_relative_path_rejects_traversal() {
    assert!(sanitize_update_relative_path(Path::new("../evil.exe")).is_err());
    assert!(sanitize_update_relative_path(Path::new("subdir/../../evil.exe")).is_err());
    assert!(sanitize_update_relative_path(Path::new("C:\\Windows\\system32\\calc.exe")).is_err());
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
    assert!(!is_valid_windows_archive_name(
        "strata-1.2.3-x86_64-pc-windows-msvc.zip",
        "1.2.3"
    ));
    assert!(!is_valid_windows_archive_name(
        "strata-1.2.3-x86_64-pc-windows-gnu.tar.gz",
        "1.2.3"
    ));
    assert!(!is_valid_windows_archive_name(
        "strata-v1.2.3-setup.exe",
        "1.2.3"
    ));
    assert!(!is_valid_windows_archive_name("malicious.zip", "1.2.3"));
    assert!(!is_valid_windows_archive_name("strata-2.0.0.zip", "1.2.3"));
    assert!(is_valid_windows_archive_name(
        &super::super::update_check::archive_name("1.2.3"),
        "1.2.3"
    ));
}
