// SPDX-License-Identifier: MIT

use super::ViewState;
use crate::{
    model::FileEntry,
    services::text_compare::{self, CompareError},
    ui::{controls, modal},
};
use gtk::{gio, glib, prelude::*};
use std::{
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

#[cfg(test)]
mod tests;

pub(super) fn supports(entries: &[FileEntry]) -> bool {
    entries.len() == 2
        && entries.iter().all(|entry| {
            entry.kind == crate::model::EntryKind::File && entry.location.native_path().is_some()
        })
}

pub(super) fn show(state: &Rc<ViewState>, entries: Vec<FileEntry>) {
    if !supports(&entries) {
        return;
    }
    let paths: Vec<_> = entries
        .iter()
        .filter_map(|entry| {
            entry
                .location
                .native_path()
                .map(std::path::Path::to_path_buf)
        })
        .collect();
    let Some(host) = modal::ModalHost::blurred_for(&state.overlay) else {
        return;
    };
    let layout = controls::modal_layout(
        crate::assets::icons::COPY,
        &crate::i18n::tr("Compare text files"),
        &crate::i18n::tr("UTF-8 text · up to 1 MiB per file · line endings normalized"),
        &crate::i18n::tr("Copy"),
    );
    layout.cancel.set_visible(false);
    layout.confirm.set_sensitive(false);
    layout.content.set_size_request(720, -1);
    let names = gtk::Label::new(Some(&format!(
        "{} ↔ {}",
        entries[0].display_name, entries[1].display_name
    )));
    names.set_wrap(true);
    layout.body.append(&names);
    let text = gtk::TextView::builder()
        .editable(false)
        .monospace(true)
        .cursor_visible(true)
        .wrap_mode(gtk::WrapMode::None)
        .build();
    text.add_css_class("preview-source-lines");
    let scroll = gtk::ScrolledWindow::builder()
        .child(&text)
        .min_content_height(320)
        .vexpand(true)
        .hexpand(true)
        .build();
    layout.body.append(&scroll);
    let status = gtk::Label::new(Some(&crate::i18n::tr("Working…")));
    status.set_wrap(true);
    layout.body.append(&status);
    layout.set_loading(true, None);
    let layer = modal::modal_layer(
        &layout.content,
        &host.overlay,
        host.blurred_root.clone(),
        None,
    );
    modal::remember_modal_focus(&layer, &host.overlay);
    host.overlay.add_overlay(&layer);
    let cancelled = Arc::new(AtomicBool::new(false));
    let cancel = cancelled.clone();
    layer.connect_sensitive_notify(move |layer| {
        if !layer.is_sensitive() {
            cancel.store(true, Ordering::Release);
        }
    });
    let cancel = cancelled.clone();
    layer.connect_unrealize(move |_| {
        cancel.store(true, Ordering::Release);
    });
    let close = {
        let layer = layer.downgrade();
        let overlay = host.overlay.clone();
        let root = host.blurred_root.clone();
        Rc::new(move || {
            if let Some(layer) = layer.upgrade() {
                modal::dismiss_modal_layer(&layer, &overlay, root.as_ref());
            }
        })
    };
    let dismiss = close.clone();
    layout.close.connect_clicked(move |_| dismiss());
    let escape = close.clone();
    modal::close_on_escape(&layer, move || escape());
    let buffer = text.buffer();
    layout.confirm.connect_clicked(move |button| {
        let (start, end) = buffer.selection_bounds().unwrap_or_else(|| buffer.bounds());
        button
            .clipboard()
            .set_text(&buffer.text(&start, &end, true));
    });
    let weak_layer = layer.downgrade();
    let weak_state = Rc::downgrade(state);
    let generation = state.browser.navigation_generation();
    let permit = text_compare::ComparisonPermit::reserve();
    let close_after_navigation = close.clone();
    let watched_state = weak_state.clone();
    let watched_layer = layer.downgrade();
    let cancel = cancelled.clone();
    glib::timeout_add_local(Duration::from_millis(100), move || {
        let navigated = watched_state
            .upgrade()
            .is_none_or(|state| state.browser.navigation_generation() != generation);
        if navigated
            || cancel.load(Ordering::Acquire)
            || watched_layer
                .upgrade()
                .is_none_or(|layer| !layer.is_sensitive())
        {
            cancel.store(true, Ordering::Release);
            if navigated {
                close();
            }
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
    glib::MainContext::default().spawn_local(async move {
        let Some(permit) = permit else {
            layout.set_loading(false, None);
            status.set_text(&crate::i18n::tr(
                "Another comparison is running. Try again shortly.",
            ));
            return;
        };
        let worker_cancel = cancelled.clone();
        let result = gio::spawn_blocking(move || {
            text_compare::compare(&permit, &paths[0], &paths[1], &worker_cancel)
        })
        .await;
        if weak_state
            .upgrade()
            .is_none_or(|state| state.browser.navigation_generation() != generation)
        {
            cancelled.store(true, Ordering::Release);
            close_after_navigation();
            return;
        }
        if cancelled.load(Ordering::Acquire)
            || weak_layer
                .upgrade()
                .is_none_or(|layer| !layer.is_sensitive())
        {
            return;
        }
        layout.set_loading(false, None);
        let message = match result {
            Ok(Ok(output)) => {
                text.buffer().set_text(&output);
                layout.confirm.set_sensitive(!output.is_empty());
                if output.is_empty() {
                    "Text is identical"
                } else {
                    "Select text to copy, or copy the whole comparison."
                }
            }
            Ok(Err(CompareError::TooLarge | CompareError::TooComplex)) => {
                "Files exceed the comparison limits."
            }
            Ok(Err(CompareError::Unsupported)) => "Choose two local UTF-8 text files.",
            Ok(Err(CompareError::Cancelled)) => return,
            Ok(Err(CompareError::Io(error))) => {
                tracing::warn!(kind = ?error.kind(), "text comparison read failed");
                "Unable to read the selected files."
            }
            Err(_) => "Unable to read the selected files.",
        };
        status.set_text(&crate::i18n::tr(message));
    });
    layer.grab_focus();
}
