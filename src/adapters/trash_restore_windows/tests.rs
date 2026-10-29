// SPDX-License-Identifier: MIT

use super::*;
#[test]
fn refuses_linux_trash_metadata_in_windows_recycle_bin() {
    assert!(decode_trashinfo_path("file:///C:/private/test").is_none());
    assert!(
        plan_restore_from_known_paths(
            Path::new("C:\\$Recycle.Bin\\file"),
            Path::new("C:\\test"),
            Path::new("C:\\$Recycle.Bin"),
            None,
            &RestoreContext::current(),
        )
        .is_err()
    );
}

#[test]
fn plans_a_restore_from_the_verified_original_path() {
    let root = tempfile::tempdir().expect("temporary root");
    let destination = root.path().join("restored.txt");
    let plan = plan_restore_from_known_paths(
        &destination,
        &destination,
        &root.path().join("recycle-scope"),
        None,
        &RestoreContext::current(),
    )
    .expect("restore plan");
    assert_eq!(plan.source_path, destination);
    assert_eq!(plan.destination, destination);
}
