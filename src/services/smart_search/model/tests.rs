// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn checksum_validation_checks_size_digest_and_cancellation() {
    let directory = tempfile::tempdir().expect("fixture");
    let path = directory.path().join("model.partial");
    fs::write(&path, b"verified fixture").expect("write");
    let digest = hex::encode(Sha256::digest(b"verified fixture"));
    let cancel = AtomicBool::new(false);
    assert!(valid(&path, 16, &digest, &cancel).expect("verified"));
    assert!(!valid(&path, 15, &digest, &cancel).expect("wrong size"));
    assert!(!valid(&path, 16, &"0".repeat(64), &cancel).expect("wrong digest"));
    cancel.store(true, Ordering::Release);
    assert!(valid(&path, 16, &digest, &cancel).is_err());
}

#[test]
fn marker_without_verified_model_files_never_enables_inference() {
    let directory = tempfile::tempdir().expect("fixture");
    let models = folder(directory.path());
    fs::create_dir_all(&models).expect("folder");
    fs::write(models.join("verified.json"), FINGERPRINT).expect("marker");
    assert!(!ready(directory.path()));
    assert!(!ready(&directory.path().join("missing")));
}
