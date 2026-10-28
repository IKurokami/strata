// SPDX-License-Identifier: MIT

use crate::{services::smart_search, ui::preferences::PreferenceManager};
use gtk::{gio, glib, prelude::*};
use std::{rc::Rc, time::Duration};

pub(super) fn page(manager: Rc<PreferenceManager>) -> gtk::Widget {
    let content = super::page_content();
    let group = super::settings_group(&content, "Smart Search");
    for (title, description, read, write) in [
        (
            "Smart Search",
            "Find related content locally. Filename and keyword search remain available when disabled.",
            PreferenceManager::smart_search_enabled as fn(&PreferenceManager) -> bool,
            PreferenceManager::set_smart_search_enabled as fn(&PreferenceManager, bool),
        ),
        (
            "Pause indexing",
            "Keep the existing index and suspend background indexing.",
            PreferenceManager::smart_search_paused,
            PreferenceManager::set_smart_search_paused,
        ),
        (
            "Index source code",
            "Include code and configuration files in a separate local index.",
            PreferenceManager::smart_search_include_code,
            PreferenceManager::set_smart_search_include_code,
        ),
        (
            "Index on battery",
            "Allow background indexing while using battery power.",
            PreferenceManager::smart_search_index_on_battery,
            PreferenceManager::set_smart_search_index_on_battery,
        ),
    ] {
        let (row, toggle) = super::settings_option(title, description, read(&manager));
        super::bindings::bind_switch(&manager, &toggle, read, write);
        group.append(&row);
    }
    let roots = super::settings_group(&content, "Indexed folders");
    super::search::tag(&roots, "Indexed folders");
    let help = gtk::Label::new(Some(&crate::i18n::tr(
        "Only folders you add are indexed. Secrets, generated files, and cloud placeholders are excluded.",
    )));
    help.set_wrap(true);
    help.set_xalign(0.0);
    help.add_css_class("settings-option-description");
    roots.append(&help);
    let folders = gtk::Box::new(gtk::Orientation::Vertical, 0);
    roots.append(&folders);
    let bound = manager.clone();
    manager.bind_preference(
        &folders,
        PreferenceManager::smart_search_roots,
        move |widget, paths| {
            let Some(container) = widget.downcast_ref::<gtk::Box>() else {
                return;
            };
            while let Some(child) = container.first_child() {
                container.remove(&child);
            }
            for path in paths {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                let name = gtk::Label::new(Some(&path.to_string_lossy()));
                name.set_xalign(0.0);
                name.set_hexpand(true);
                name.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
                row.append(&name);
                let remove = gtk::Button::with_label(&crate::i18n::tr("Remove"));
                let manager = bound.clone();
                remove.connect_clicked(move |_| {
                    let mut roots = manager.smart_search_roots();
                    roots.retain(|root| root != &path);
                    manager.set_smart_search_roots(roots);
                });
                row.append(&remove);
                container.append(&row);
            }
        },
    );
    let add = gtk::Button::with_label(&crate::i18n::tr("Add folder"));
    add.add_css_class("settings-action-button");
    roots.append(&add);
    let selected = manager.clone();
    add.connect_clicked(move |button| {
        let Some(window) = button.root().and_downcast::<gtk::Window>() else {
            return;
        };
        let manager = selected.clone();
        gtk::FileDialog::builder()
            .title(crate::i18n::tr("Add folder"))
            .modal(true)
            .build()
            .select_folder(Some(&window), gio::Cancellable::NONE, move |result| {
                if let Ok(file) = result
                    && let Some(path) = file.path()
                {
                    let mut roots = manager.smart_search_roots();
                    roots.push(path);
                    manager.set_smart_search_roots(roots);
                }
            });
    });
    let models = super::settings_group(&content, "Local search models");
    let consent = gtk::Label::new(Some(&crate::i18n::tr(
        "Download 865 MB of verified models to enable local semantic search. Documents stay on this computer.",
    )));
    consent.set_xalign(0.0);
    consent.set_wrap(true);
    consent.add_css_class("settings-option-description");
    models.append(&consent);
    let download = gtk::Button::with_label(&crate::i18n::tr("Download models"));
    download.add_css_class("settings-action-button");
    download.connect_clicked(|_| smart_search::download_models());
    models.append(&download);
    let cancel = gtk::Button::with_label(&crate::i18n::tr("Cancel"));
    cancel.connect_clicked(|_| smart_search::cancel_download());
    models.append(&cancel);
    let status = gtk::Label::new(None);
    status.set_wrap(true);
    status.set_xalign(0.0);
    models.append(&status);
    let progress = gtk::ProgressBar::new();
    models.append(&progress);
    for widget in [
        consent.upcast_ref::<gtk::Widget>(),
        download.upcast_ref(),
        cancel.upcast_ref(),
        status.upcast_ref(),
        progress.upcast_ref(),
    ] {
        super::search::tag(widget, "Local search models");
    }
    let weak = status.downgrade();
    let weak_download = download.downgrade();
    let weak_progress = progress.downgrade();
    glib::timeout_add_local(Duration::from_millis(500), move || {
        let (Some(label), Some(button), Some(progress)) = (
            weak.upgrade(),
            weak_download.upgrade(),
            weak_progress.upgrade(),
        ) else {
            return glib::ControlFlow::Break;
        };
        let state = smart_search::status();
        button.set_sensitive(!state.downloading && !state.models_ready);
        let message = if state.failed {
            "Search setup needs attention"
        } else if state.downloading {
            "Downloading models…"
        } else if state.running {
            "Indexing files…"
        } else if state.models_ready {
            "Models ready"
        } else {
            "Keyword search ready; models not installed"
        };
        label.set_text(&format!(
            "{}\n{}",
            crate::i18n::tr(message),
            rust_i18n::t!(
                "%{indexed} indexed · %{skipped} skipped",
                indexed = state.indexed,
                skipped = state.skipped
            )
        ));
        progress.set_visible(state.downloading);
        progress.set_fraction(state.downloaded as f64 / state.total.max(1) as f64);
        glib::ControlFlow::Continue
    });
    let rebuild = gtk::Button::with_label(&crate::i18n::tr("Rebuild content index"));
    rebuild.add_css_class("settings-action-button");
    models.append(&rebuild);
    super::search::tag(&rebuild, "Rebuild content index");
    rebuild.connect_clicked(|button|{
        let Some(window)=button.root().and_downcast::<gtk::Window>()else{return};
        let dialog=gtk::AlertDialog::builder().message(crate::i18n::tr("Rebuild content index?")).detail(crate::i18n::tr("The local search index will be deleted and rebuilt. Your files and downloaded models are kept.")).buttons([crate::i18n::tr("Cancel"),crate::i18n::tr("Rebuild")]).cancel_button(0).default_button(0).build();
        dialog.choose(Some(&window),gio::Cancellable::NONE,|result|{if matches!(result,Ok(1)){smart_search::rebuild_index();}});
    });
    let clear = gtk::Button::with_label(&crate::i18n::tr("Delete content index"));
    clear.add_css_class("settings-action-button");
    models.append(&clear);
    super::search::tag(&clear, "Delete content index");
    clear.connect_clicked(move |button| {
        let Some(window) = button.root().and_downcast::<gtk::Window>() else {
            return;
        };
        let manager = manager.clone();
        gtk::AlertDialog::builder()
            .message(crate::i18n::tr("Delete content index?"))
            .detail(crate::i18n::tr(
                "Only the local content index is removed. Indexing pauses until you resume it.",
            ))
            .buttons([crate::i18n::tr("Cancel"), crate::i18n::tr("Delete")])
            .cancel_button(0)
            .default_button(0)
            .build()
            .choose(Some(&window), gio::Cancellable::NONE, move |result| {
                if matches!(result, Ok(1)) {
                    manager.set_smart_search_paused(true);
                    smart_search::rebuild_index();
                }
            });
    });
    let skipped = gtk::Button::with_label(&crate::i18n::tr("Show skipped files"));
    skipped.add_css_class("settings-action-button");
    models.append(&skipped);
    let report = gtk::Label::new(None);
    report.set_xalign(0.0);
    report.set_wrap(true);
    report.set_selectable(true);
    models.append(&report);
    let weak_report = report.downgrade();
    skipped.connect_clicked(move |_| {
        let receiver = smart_search::skipped_report();
        let weak = weak_report.clone();
        glib::timeout_add_local(Duration::from_millis(50), move || {
            let Some(label) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            match receiver.try_recv() {
                Ok(rows) => {
                    label.set_text(
                        &rows
                            .into_iter()
                            .map(|(path, reason)| {
                                format!(
                                    "{path} — {}",
                                    crate::i18n::tr(match reason.as_str() {
                                        "models-not-ready" => "Models are required for this file",
                                        "unsupported-format" => "Unsupported file format",
                                        _ => "File could not be indexed",
                                    })
                                )
                            })
                            .collect::<Vec<_>>()
                            .join("\n"),
                    );
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(_) => glib::ControlFlow::Break,
            }
        });
    });
    super::scrollable_page(&content, None)
}
