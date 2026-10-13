// SPDX-License-Identifier: MIT

//! Windows folder selection uses GTK's native platform dialog, not the Linux
//! xdg-desktop-portal backend or an extra WebKit/chooser window. It starts
//! only when requested; no startup shell, volume enumeration or UAC prompt.

use std::{
    path::{Path, PathBuf},
    rc::Rc,
};

use gtk::{gio, glib, prelude::*};

pub(crate) struct DestinationRequest {
    pub parent: gtk::Window,
    pub title: String,
    pub accept_label: String,
    pub initial_directory: PathBuf,
    pub root_limit: Option<PathBuf>,
    pub allow_create: bool,
    pub validate: Rc<dyn Fn(&Path) -> Result<PathBuf, String>>,
}

fn within_directory(root: &Path, path: &Path) -> bool {
    // Realpath is needed: Windows junctions and directory symlinks may escape
    // a USB/removable volume despite an innocuous lexical path prefix.
    // Only run on a background thread AFTER the user selects a destination.
    match (root.canonicalize(), path.canonicalize()) {
        (Ok(root), Ok(path)) => path.starts_with(root),
        _ => false,
    }
}

pub(crate) fn present_destination_chooser(
    destination: DestinationRequest,
    completion: impl FnOnce(PathBuf) + 'static,
) {
    let dialog = gtk::FileDialog::new();
    dialog.set_modal(true);
    dialog.set_title(&destination.title);
    dialog.set_accept_label(Some(&destination.accept_label));
    dialog.set_initial_folder(Some(&gio::File::for_path(&destination.initial_directory)));

    // GtkFileDialog delegates to the native Windows file-picker and supports
    // cancellation without any Strata-owned subprocess. The operating-system
    // chooser controls whether New Folder is displayed; do not synthesize a
    // separate creation path when allow_create is false.
    let _allow_create = destination.allow_create;
    let _task = glib::MainContext::default().spawn_local(async move {
        let Ok(file) = dialog.select_folder_future(Some(&destination.parent)).await else {
            return;
        };
        let Some(selected) = file.path() else {
            return;
        };
        if !selected.is_absolute() {
            return;
        }
        if let Some(root) = destination.root_limit {
            let check = gio::spawn_blocking(move || within_directory(&root, &selected)).await;
            // Select path again from the GFile after moving the PathBuf to
            // a background worker, preserving the real path without UTF-8 loss.
            if !matches!(check, Ok(true)) {
                return;
            }
        }
        let Some(path) = file.path() else {
            return;
        };
        if let Ok(accepted) = (destination.validate)(&path) {
            completion(accepted);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn root_confinement_rejects_sibling_and_missing_paths() {
        let root = tempfile::tempdir().unwrap();
        let allowed = root.path().join("inside");
        std::fs::create_dir(&allowed).unwrap();
        assert!(within_directory(root.path(), &allowed));
        assert!(!within_directory(root.path(), &root.path().join("missing")));
        let sibling = tempfile::tempdir().unwrap();
        assert!(!within_directory(root.path(), sibling.path()));
    }
}
