// SPDX-License-Identifier: MIT

use std::{path::PathBuf, rc::Rc};

use gtk::{gio, glib, prelude::*};

use super::ViewState;
use crate::{
    model::{EntryKind, FileEntry},
    sandbox::Cancellation,
    services::image_exports::{Export, ExportError},
    ui::{controls, modal},
};

fn paths(entries: &[FileEntry], export: Export) -> Option<Vec<PathBuf>> {
    if entries.len() > crate::services::image_exports::MAX_SHEET_IMAGES {
        return None;
    }
    let paths: Vec<_> = entries
        .iter()
        .map(|entry| {
            let path = entry.location.native_path()?;
            let extension = path.extension()?.to_str()?.to_ascii_lowercase();
            let supported = match export {
                Export::Png => matches!(
                    extension.as_str(),
                    "jpg" | "jpeg" | "jpe" | "bmp" | "gif" | "webp"
                ),
                Export::ContactSheet => {
                    matches!(extension.as_str(), "png" | "jpg" | "jpeg" | "jpe" | "webp")
                }
            };
            (supported && entry.kind == EntryKind::File).then(|| path.to_owned())
        })
        .collect::<Option<_>>()?;
    export.destination(&paths)?;
    Some(paths)
}

pub(super) fn supports(entries: &[FileEntry], export: Export) -> bool {
    paths(entries, export).is_some()
}

pub(super) fn show(state: &Rc<ViewState>, entries: &[FileEntry], export: Export) {
    let Some(paths) = paths(entries, export) else {
        return;
    };
    let Some(host) = modal::ModalHost::blurred_for(&state.overlay) else {
        return;
    };
    let title = match export {
        Export::Png => "Convert image to PNG",
        Export::ContactSheet => "Create PNG contact sheet",
    };
    let subtitle = match export {
        Export::Png => "Save a PNG copy beside the original.",
        Export::ContactSheet => "Arrange the selected images in a PNG grid beside the originals.",
    };
    let layout = controls::modal_layout(
        crate::assets::icons::COPY_PLUS,
        &crate::i18n::tr(title),
        &crate::i18n::tr(subtitle),
        &crate::i18n::tr("Save"),
    );
    let destination = export.destination(&paths).expect("validated export paths");
    let output_name = destination
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    let name = gtk::Label::new(Some(&output_name));
    name.set_wrap(true);
    layout.body.append(&name);
    let status = gtk::Label::new(None);
    status.set_wrap(true);
    layout.body.append(&status);

    let layer = modal::modal_layer(
        &layout.content,
        &host.overlay,
        host.blurred_root.clone(),
        None,
    );
    modal::remember_modal_focus(&layer, &host.overlay);
    host.overlay.add_overlay(&layer);
    let cancel = Cancellation::default();
    let on_unrealize = cancel.clone();
    layer.connect_unrealize(move |_| on_unrealize.cancel());
    let on_dismiss = cancel.clone();
    layer.connect_sensitive_notify(move |layer| {
        if !layer.is_sensitive() {
            on_dismiss.cancel();
        }
    });
    let on_close = cancel.clone();
    let close = Rc::new({
        let weak = layer.downgrade();
        let overlay = host.overlay.downgrade();
        let root = host.blurred_root.as_ref().map(ObjectExt::downgrade);
        move || {
            on_close.cancel();
            if let (Some(layer), Some(overlay)) = (weak.upgrade(), overlay.upgrade()) {
                let root = root.as_ref().and_then(glib::WeakRef::upgrade);
                modal::dismiss_modal_layer(&layer, &overlay, root.as_ref());
            }
        }
    });
    for button in [&layout.close, &layout.cancel] {
        let close = close.clone();
        button.connect_clicked(move |_| close());
    }
    modal::close_on_escape(&layer, move || close());

    let layer = layer.downgrade();
    layout.confirm.connect_clicked(move |button| {
        if !button.is_sensitive() {
            return;
        }
        button.set_sensitive(false);
        status.set_text(&crate::i18n::tr("Working…"));
        let paths = paths.clone();
        let status = status.clone();
        let layer = layer.clone();
        let worker_cancel = cancel.clone();
        let button = button.downgrade();
        glib::MainContext::default().spawn_local(async move {
            let result = gio::spawn_blocking(move || export.run(&paths, &worker_cancel)).await;
            if layer.upgrade().is_none_or(|layer| !layer.is_sensitive()) {
                return;
            }
            let message = match result {
                Ok(Ok(())) => "Done",
                Ok(Err(ExportError::Cancelled)) => return,
                Ok(Err(ExportError::Exists)) => "An output file already exists.",
                Ok(Err(ExportError::Busy)) => "Another image export is running.",
                Ok(Err(ExportError::Unavailable)) | Err(_) => {
                    "Unable to export the selected images."
                }
            };
            if !matches!(result, Ok(Ok(())))
                && let Some(button) = button.upgrade()
            {
                button.set_sensitive(true);
            }
            status.set_text(&crate::i18n::tr(message));
        });
    });
}
