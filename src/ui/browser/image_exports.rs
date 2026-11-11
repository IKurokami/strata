// SPDX-License-Identifier: MIT

use std::{path::PathBuf, rc::Rc};

use gtk::{gio, glib, prelude::*};

use super::ViewState;
use crate::{
    model::{EntryKind, FileEntry},
    sandbox::Cancellation,
    services::image_exports::{DEFAULT_SHEET_NAME, Export, ExportError, SheetLayout},
    ui::{controls, modal},
};

fn paths(entries: &[FileEntry], export: Export) -> Option<Vec<PathBuf>> {
    if entries.is_empty() || entries.len() > crate::services::image_exports::MAX_EXPORT_IMAGES {
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
                Export::WebP => matches!(
                    extension.as_str(),
                    "png" | "jpg" | "jpeg" | "jpe" | "bmp" | "gif"
                ),
                Export::ContactSheet => {
                    matches!(extension.as_str(), "png" | "jpg" | "jpeg" | "jpe" | "webp")
                }
            };
            (supported && entry.kind == EntryKind::File).then(|| path.to_owned())
        })
        .collect::<Option<_>>()?;
    if matches!(export, Export::ContactSheet) {
        export.destination(&paths, DEFAULT_SHEET_NAME)?;
    }
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
    let batch = paths.len() > 1 && !matches!(export, Export::ContactSheet);
    let title = match (export, batch) {
        (Export::Png, false) => "Convert image to PNG",
        (Export::WebP, false) => "Convert image to WebP",
        (Export::Png, true) => "Convert images to PNG",
        (Export::WebP, true) => "Convert images to WebP",
        (Export::ContactSheet, _) => "Create PNG contact sheet",
    };
    let subtitle = match (export, batch) {
        (_, true) => "Save converted copies beside their originals.",
        (Export::Png, false) => "Save a PNG copy beside the original.",
        (Export::WebP, false) => "Save a lossless WebP copy beside the original.",
        (Export::ContactSheet, false) => "Save the arranged images as a PNG beside the originals.",
    };
    let layout = controls::modal_layout(
        crate::assets::icons::COPY_PLUS,
        &crate::i18n::tr(title),
        &crate::i18n::tr(subtitle),
        &crate::i18n::tr("Save"),
    );
    let sheet_options = if matches!(export, Export::ContactSheet) {
        let options = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let name = controls::form_entry();
        name.set_text(DEFAULT_SHEET_NAME);
        let name_label = controls::form_label(&crate::i18n::tr("Name"));
        name_label.set_mnemonic_widget(Some(&name));
        options.append(&name_label);
        options.append(&name);
        let (arrangement, buttons) =
            controls::segmented_control(&["Grid layout", "Single row", "Single column"], 0);
        options.append(&arrangement);
        layout.body.append(&options);
        Some((options, name, buttons))
    } else if !batch {
        let destination = export
            .destination(&paths, DEFAULT_SHEET_NAME)
            .expect("validated export paths");
        let output_name = destination
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        let name = gtk::Label::new(Some(&output_name));
        name.set_wrap(true);
        layout.body.append(&name);
        None
    } else {
        None
    };
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
        let (sheet_name, arrangement) = sheet_options.as_ref().map_or_else(
            || (DEFAULT_SHEET_NAME.to_owned(), SheetLayout::Grid),
            |(_, name, buttons)| {
                let selected = [SheetLayout::Grid, SheetLayout::Row, SheetLayout::Column]
                    .into_iter()
                    .zip(buttons)
                    .find_map(|(layout, button)| button.is_active().then_some(layout))
                    .unwrap_or_default();
                (name.text().to_string(), selected)
            },
        );
        if !batch && export.destination(&paths, &sheet_name).is_none() {
            status.set_text(&crate::i18n::tr("Enter a valid filename ending in .png."));
            return;
        }
        let options = sheet_options.as_ref().map(|(options, _, _)| {
            options.set_sensitive(false);
            options.downgrade()
        });
        button.set_sensitive(false);
        status.set_text(&crate::i18n::tr("Working…"));
        let paths = paths.clone();
        let total = paths.len();
        let status = status.clone();
        let layer = layer.clone();
        let worker_cancel = cancel.clone();
        let button = button.downgrade();
        glib::MainContext::default().spawn_local(async move {
            let result = gio::spawn_blocking(move || {
                if batch {
                    export.run_batch(&paths, &worker_cancel)
                } else {
                    export
                        .run(&paths, &sheet_name, arrangement, &worker_cancel)
                        .map_err(|error| (0, error))
                }
            })
            .await;
            if layer.upgrade().is_none_or(|layer| !layer.is_sensitive()) {
                return;
            }
            let (saved, message) = match result {
                Ok(Ok(())) => (total, "Done"),
                Ok(Err((_, ExportError::Cancelled))) => return,
                Ok(Err((saved, ExportError::Exists))) => (saved, "An output file already exists."),
                Ok(Err((saved, ExportError::Busy))) => (saved, "Another image export is running."),
                Ok(Err((saved, ExportError::Unavailable))) => {
                    (saved, "Unable to export the selected images.")
                }
                Err(_) => (0, "Unable to export the selected images."),
            };
            if saved == 0 {
                if let Some(button) = button.upgrade() {
                    button.set_sensitive(true);
                }
                if let Some(options) = options.and_then(|options| options.upgrade()) {
                    options.set_sensitive(true);
                }
            }
            let message = crate::i18n::tr(message);
            if saved > 0 && saved < total {
                let progress = rust_i18n::t!(
                    "Images saved: %{saved}/%{total}.",
                    saved = saved,
                    total = total
                );
                status.set_text(&format!("{progress}\n{message}"));
            } else {
                status.set_text(&message);
            }
        });
    });
}
