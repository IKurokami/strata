// SPDX-License-Identifier: MIT

use crate::{
    assets::{icons, primary_icon},
    services::smart_search,
    ui::preferences::PreferenceManager,
};
use gtk::{gio, glib, prelude::*};
use std::{path::Path, rc::Rc, time::Duration};

pub(super) fn page(manager: Rc<PreferenceManager>) -> gtk::Widget {
    let content = super::page_content();
    content.add_css_class("smart-settings-page");

    let main = super::settings_group(&content, "");
    let (row, enabled) = super::settings_option(
        "Smart Search",
        "",
        manager.smart_search_enabled(),
    );
    super::bindings::bind_switch(
        &manager,
        &enabled,
        PreferenceManager::smart_search_enabled,
        PreferenceManager::set_smart_search_enabled,
    );
    main.append(&row);

    let details = gtk::Box::new(gtk::Orientation::Vertical, 0);
    details.add_css_class("smart-settings-details");
    content.append(&details);
    manager.bind_preference(
        &details,
        PreferenceManager::smart_search_enabled,
        |widget, active| widget.set_visible(active),
    );

    let folders = super::settings_group(&details, "Indexed folders");
    folders.add_css_class("smart-settings-folder-group");
    let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
    folders.append(&list);

    let add = action_button("Add folder", icons::PLUS);
    add.add_css_class("smart-settings-primary");
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    actions.add_css_class("smart-settings-actions");
    actions.append(&add);
    folders.append(&actions);

    let remove_from = manager.clone();
    manager.bind_preference(
        &list,
        PreferenceManager::smart_search_roots,
        move |widget, paths| {
            let Some(list) = widget.downcast_ref::<gtk::Box>() else {
                return;
            };
            while let Some(child) = list.first_child() {
                list.remove(&child);
            }
            list.set_visible(!paths.is_empty());
            for path in paths {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
                row.add_css_class("smart-settings-folder");
                let icon = primary_icon(icons::FOLDER, 18);
                icon.set_valign(gtk::Align::Center);
                row.append(&icon);
                let label = gtk::Label::new(Some(&display_folder(&path)));
                label.add_css_class("smart-settings-path");
                label.set_xalign(0.0);
                label.set_hexpand(true);
                label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
                row.append(&label);
                let remove = gtk::Button::builder()
                    .child(&primary_icon(icons::X, 16))
                    .tooltip_text(crate::i18n::tr("Remove"))
                    .valign(gtk::Align::Center)
                    .build();
                remove.add_css_class("smart-settings-remove");
                crate::ui::accessibility::set_label(
                    &remove,
                    &crate::i18n::tr("Remove"),
                );
                let manager = remove_from.clone();
                remove.connect_clicked(move |_| {
                    let mut roots = manager.smart_search_roots();
                    roots.retain(|root| root != &path);
                    manager.set_smart_search_roots(roots);
                });
                row.append(&remove);
                list.append(&row);
            }
        },
    );

    let add_to = manager.clone();
    add.connect_clicked(move |button| {
        let Some(window) = button.root().and_downcast::<gtk::Window>() else {
            return;
        };
        let manager = add_to.clone();
        gtk::FileDialog::builder()
            .title(crate::i18n::tr("Add folder"))
            .modal(true)
            .build()
            .select_folder(Some(&window), gio::Cancellable::NONE, move |result| {
                if let Ok(file) = result
                    && let Some(path) = file.path()
                {
                    let mut roots = manager.smart_search_roots();
                    if !roots.contains(&path) {
                        roots.push(path);
                        manager.set_smart_search_roots(roots);
                    }
                }
            });
    });

    let models = super::settings_group(&details, "");
    models.add_css_class("smart-settings-model");
    super::search::tag(&models, "Local search models");
    let model_row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    model_row.add_css_class("smart-settings-model-row");
    let model_icon = primary_icon(icons::DOWNLOADS, 18);
    model_row.append(&model_icon);
    let model_label = gtk::Label::new(None);
    model_label.set_xalign(0.0);
    model_label.set_hexpand(true);
    model_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    model_label.add_css_class("settings-option-description");
    model_row.append(&model_label);
    let download = action_button("Download models", icons::DOWNLOADS);
    download.add_css_class("smart-settings-primary");
    let cancel = action_button("Cancel", icons::X);
    model_row.append(&download);
    model_row.append(&cancel);
    models.append(&model_row);
    let progress = gtk::ProgressBar::new();
    progress.add_css_class("smart-settings-progress");
    models.append(&progress);

    download.connect_clicked(|button| {
        let Some(window) = button.root().and_downcast::<gtk::Window>() else {
            return;
        };
        gtk::AlertDialog::builder()
            .message(crate::i18n::tr("Download models"))
            .detail(crate::i18n::tr(
                "Download 865 MB of verified models to enable local semantic search. Documents stay on this computer.",
            ))
            .buttons([
                crate::i18n::tr("Cancel"),
                crate::i18n::tr("Download models"),
            ])
            .cancel_button(0)
            .default_button(1)
            .build()
            .choose(Some(&window), gio::Cancellable::NONE, |result| {
                if matches!(result, Ok(1)) {
                    smart_search::download_models();
                }
            });
    });
    cancel.connect_clicked(|_| smart_search::cancel_download());

    let indexing = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    indexing.add_css_class("smart-settings-indexing");
    let spinner = gtk::Spinner::new();
    spinner.set_valign(gtk::Align::Center);
    indexing.append(&spinner);
    let indexing_label = gtk::Label::new(Some(&crate::i18n::tr("Indexing files…")));
    indexing_label.add_css_class("settings-option-description");
    indexing.append(&indexing_label);
    details.append(&indexing);

    let advanced = gtk::Expander::new(Some(&crate::i18n::tr("More options")));
    advanced.add_css_class("smart-settings-advanced");
    advanced.set_expanded(manager.smart_search_paused());
    let advanced_group = gtk::Box::new(gtk::Orientation::Vertical, 0);
    advanced_group.add_css_class("settings-group");
    for (title, description, read, write) in [
        (
            "Pause indexing",
            "Keep the existing index and suspend background indexing.",
            PreferenceManager::smart_search_paused as fn(&PreferenceManager) -> bool,
            PreferenceManager::set_smart_search_paused as fn(&PreferenceManager, bool),
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
        advanced_group.append(&row);
    }
    let rebuild = action_button("Rebuild content index", icons::REFRESH);
    rebuild.add_css_class("smart-settings-subtle");
    super::search::tag(&rebuild, "Rebuild content index");
    let rebuild_row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    rebuild_row.add_css_class("smart-settings-advanced-actions");
    rebuild_row.append(&rebuild);
    advanced_group.append(&rebuild_row);
    rebuild.connect_clicked(|button| {
        let Some(window) = button.root().and_downcast::<gtk::Window>() else {
            return;
        };
        gtk::AlertDialog::builder()
            .message(crate::i18n::tr("Rebuild content index?"))
            .detail(crate::i18n::tr(
                "The local search index will be deleted and rebuilt. Your files and downloaded models are kept.",
            ))
            .buttons([crate::i18n::tr("Cancel"), crate::i18n::tr("Rebuild")])
            .cancel_button(0)
            .default_button(0)
            .build()
            .choose(Some(&window), gio::Cancellable::NONE, |result| {
                if matches!(result, Ok(1)) {
                    smart_search::rebuild_index();
                }
            });
    });
    advanced.set_child(Some(&advanced_group));
    details.append(&advanced);

    let state = smart_search::status();
    refresh_status(
        &state, &manager, &models, &model_label, &download, &cancel, &progress,
        &indexing, &spinner, &rebuild, &rebuild_row,
    );
    let weak_models = models.downgrade();
    let weak_label = model_label.downgrade();
    let weak_download = download.downgrade();
    let weak_cancel = cancel.downgrade();
    let weak_progress = progress.downgrade();
    let weak_indexing = indexing.downgrade();
    let weak_spinner = spinner.downgrade();
    let weak_rebuild = rebuild.downgrade();
    let weak_rebuild_row = rebuild_row.downgrade();
    let manager = manager.clone();
    glib::timeout_add_local(Duration::from_millis(500), move || {
        let (
            Some(models), Some(label), Some(download), Some(cancel),
            Some(progress), Some(indexing), Some(spinner), Some(rebuild),
            Some(rebuild_row),
        ) = (
            weak_models.upgrade(), weak_label.upgrade(), weak_download.upgrade(),
            weak_cancel.upgrade(), weak_progress.upgrade(), weak_indexing.upgrade(),
            weak_spinner.upgrade(), weak_rebuild.upgrade(), weak_rebuild_row.upgrade(),
        ) else {
            return glib::ControlFlow::Break;
        };
        refresh_status(
            &smart_search::status(), &manager, &models, &label, &download, &cancel,
            &progress, &indexing, &spinner, &rebuild, &rebuild_row,
        );
        glib::ControlFlow::Continue
    });
    super::scrollable_page(&content, None)
}

fn action_button(label: &str, icon: &str) -> gtk::Button {
    let button = gtk::Button::new();
    button.add_css_class("smart-settings-action");
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 7);
    content.append(&primary_icon(icon, 16));
    content.append(&gtk::Label::new(Some(&crate::i18n::tr(label))));
    button.set_child(Some(&content));
    button
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ModelStage {
    NeedsDownload,
    Downloading,
    Ready,
    Failed,
}

fn model_stage(status: &smart_search::Status) -> ModelStage {
    if status.downloading {
        ModelStage::Downloading
    } else if status.failed {
        ModelStage::Failed
    } else if status.models_ready {
        ModelStage::Ready
    } else {
        ModelStage::NeedsDownload
    }
}

#[expect(clippy::too_many_arguments, reason = "One bounded visual-state update for the Settings page")]
fn refresh_status(
    status: &smart_search::Status,
    manager: &PreferenceManager,
    models: &gtk::Box,
    model_label: &gtk::Label,
    download: &gtk::Button,
    cancel: &gtk::Button,
    progress: &gtk::ProgressBar,
    indexing: &gtk::Box,
    spinner: &gtk::Spinner,
    rebuild: &gtk::Button,
    rebuild_row: &gtk::Box,
) {
    let stage = model_stage(status);
    models.set_visible(stage != ModelStage::Ready);
    download.set_visible(matches!(stage, ModelStage::NeedsDownload | ModelStage::Failed) && !status.models_ready);
    cancel.set_visible(stage == ModelStage::Downloading);
    progress.set_visible(stage == ModelStage::Downloading);
    if stage == ModelStage::Downloading {
        model_label.set_text(&crate::i18n::tr("Downloading models…"));
        progress.set_fraction(
            (status.downloaded as f64 / status.total.max(1) as f64).clamp(0.0, 1.0),
        );
    } else if stage == ModelStage::Failed {
        model_label.set_text(&crate::i18n::tr("Search setup needs attention"));
    } else {
        model_label.set_text("");
    }
    model_label.set_visible(matches!(stage, ModelStage::Downloading | ModelStage::Failed));
    let running = status.running && !manager.smart_search_paused();
    indexing.set_visible(running);
    if running {
        spinner.start();
    } else {
        spinner.stop();
    }
    let can_rebuild = status.models_ready && !status.downloading && !running
        && (status.indexed > 0 || status.failed);
    rebuild.set_visible(can_rebuild);
    rebuild_row.set_visible(can_rebuild);
}

fn display_folder(path: &Path) -> String {
    let raw = path.to_string_lossy();
    for (prefix, replacement) in [
        (r"\\?\UNC\", r"\\"),
        ("//?/UNC/", "//"),
        (r"\\?\", ""),
        ("//?/", ""),
    ] {
        if let Some(rest) = raw.strip_prefix(prefix) {
            return format!("{replacement}{rest}");
        }
    }
    raw.into_owned()
}

#[cfg(test)]
mod tests;
