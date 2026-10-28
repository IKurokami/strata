// SPDX-License-Identifier: MIT

use super::*;

fn status(models_ready: bool, downloading: bool, failed: bool) -> smart_search::Status {
    smart_search::Status {
        models_ready,
        downloading,
        downloaded: 0,
        total: 100,
        indexed: 0,
        running: false,
        failed,
        folder_counts: std::collections::HashMap::new(),
    }
}

#[test]
fn models_only_offer_download_when_missing_and_cancel_while_downloading() {
    assert_eq!(
        model_stage(&status(false, false, false)),
        ModelStage::NeedsDownload
    );
    assert_eq!(
        model_stage(&status(false, true, false)),
        ModelStage::Downloading
    );
    assert_eq!(model_stage(&status(true, false, false)), ModelStage::Ready);
    assert_eq!(model_stage(&status(false, false, true)), ModelStage::Failed);
}

#[test]
fn displaying_verbatim_windows_folders_does_not_change_native_identity() {
    let path = Path::new(r"\\?\C:\Users\Public\Documents");
    assert_eq!(display_folder(path), r"C:\Users\Public\Documents");
    assert_eq!(
        display_folder(Path::new(r"\\?\UNC\server\share\folder")),
        r"\\server\share\folder"
    );
    assert_eq!(
        display_folder(Path::new("//?/C:/Users/Public")),
        "C:/Users/Public"
    );
}

#[test]
fn folder_indexed_count_resolves_verbatim_and_standard_paths() {
    let mut state = status(true, false, false);
    state
        .folder_counts
        .insert(PathBuf::from(r"C:\Users\Public\Videos"), 7);
    assert_eq!(
        state.folder_indexed_count(Path::new(r"\\?\C:\Users\Public\Videos")),
        7
    );
    assert_eq!(
        state.folder_indexed_count(Path::new(r"C:\Users\Public\Documents")),
        0
    );
}
