// SPDX-License-Identifier: MIT

use std::{cell::RefCell, rc::Rc};

use gtk::prelude::*;

use crate::{
    app::{Browser, BrowserEvent},
    model::{EntryKind, FileEntry, Location, MetadataValue},
    services::{NavigationHistory, SearchItem},
    ui::{
        browser::BrowserView, preferences::PreferenceManager, preview::PreviewDrawer,
        search::SearchDialog,
    },
};

use super::WindowContent;

pub(super) fn install(
    window: &gtk::ApplicationWindow,
    content: &WindowContent,
    preferences: &Rc<PreferenceManager>,
) {
    let controller = content.browser.browser();
    let history = NavigationHistory::shared();
    install_history_recorder(&controller, &history);
    let preview = content.preview.clone();
    let search_preferences = preferences.clone();
    let activated_browser = content.browser.downgrade();
    let activate = Rc::new(move |item| {
        if let Some(browser) = activated_browser.upgrade() {
            activate_result(&browser, &preview, &search_preferences, item);
        }
    });
    let dismissed_root = content.blurred_root.downgrade();
    let dismissed_button = content.header.search.downgrade();
    let dismiss = Rc::new(move || {
        if let Some(root) = dismissed_root.upgrade() {
            root.set_blurred(false);
        }
        if let Some(button) = dismissed_button.upgrade() {
            button.remove_css_class("active");
        }
    });
    let browser = content.browser.downgrade();
    let preview = content.preview.clone();
    let reveal = Rc::new(move |item: SearchItem| {
        preview.clear_target();
        if let Some(browser) = browser.upgrade() {
            browser.reveal_location(Location::local(item.path));
        }
    });
    let launcher = Rc::new(SearchLauncher {
        dialog: RefCell::new(None),
        overlay: content.overlay.clone(),
        activate,
        reveal,
        dismiss,
    });
    let toggle = toggle_handler(launcher.clone(), content, preferences);
    let clicked_search = toggle.clone();
    content.header.search.connect_clicked(move |_| {
        if crate::ui::tenxer_mode::chrome_suppressed() {
            return;
        }
        clicked_search();
    });
    let search_layer = launcher.clone();
    super::add_guarded_window_action(
        window,
        &content.actions,
        "search",
        &content.browser,
        move || search_layer.widget(),
        move || toggle(),
    );

    let jump_layer = launcher.clone();
    let jump = folder_jump_handler(launcher, content, history);
    super::add_guarded_window_action(
        window,
        &content.actions,
        "jump-folder",
        &content.browser,
        move || jump_layer.widget(),
        move || jump(),
    );
}

struct SearchLauncher {
    dialog: RefCell<Option<SearchDialog>>,
    overlay: gtk::Overlay,
    activate: Rc<dyn Fn(SearchItem)>,
    reveal: Rc<dyn Fn(SearchItem)>,
    dismiss: Rc<dyn Fn()>,
}

impl SearchLauncher {
    fn dialog(&self) -> SearchDialog {
        if let Some(dialog) = self.dialog.borrow().clone() {
            return dialog;
        }
        let dialog = SearchDialog::new(
            self.activate.clone(),
            self.reveal.clone(),
            self.dismiss.clone(),
        );
        self.overlay.add_overlay(&dialog.widget());
        self.dialog.borrow_mut().replace(dialog.clone());
        dialog
    }

    fn widget(&self) -> Option<gtk::Widget> {
        self.dialog.borrow().as_ref().map(SearchDialog::widget)
    }

    fn is_visible(&self) -> bool {
        self.dialog
            .borrow()
            .as_ref()
            .is_some_and(SearchDialog::is_visible)
    }
}

#[derive(Default)]
struct VisitRecorder {
    // Failed loads stay pending so a later successful retry records the visit.
    pending: Vec<Option<Location>>,
}

impl VisitRecorder {
    fn handle(&mut self, event: &BrowserEvent) -> Option<std::path::PathBuf> {
        match event {
            BrowserEvent::Reset => self.pending.clear(),
            BrowserEvent::ColumnsTruncated { len, .. } => self.pending.truncate(*len),
            BrowserEvent::ColumnAdded { depth, location } => {
                if self.pending.len() <= *depth {
                    self.pending.resize(depth + 1, None);
                }
                self.pending[*depth] = Some(location.clone());
            }
            BrowserEvent::LoadFinished { depth, .. } => {
                return self
                    .pending
                    .get_mut(*depth)
                    .and_then(Option::take)
                    .and_then(|location| location.native_path().map(std::path::Path::to_path_buf));
            }
            _ => {}
        }
        None
    }
}

fn install_history_recorder(controller: &Rc<Browser>, history: &Rc<NavigationHistory>) {
    let recorder = Rc::new(RefCell::new(VisitRecorder::default()));
    let history = history.clone();
    controller.observe(move |event| {
        if let Some(path) = recorder.borrow_mut().handle(event) {
            history.record(&path);
        }
    });
}

fn toggle_handler(
    launcher: Rc<SearchLauncher>,
    content: &WindowContent,
    preferences: &Rc<PreferenceManager>,
) -> Rc<dyn Fn()> {
    let button = content.header.search.downgrade();
    let root = content.blurred_root.downgrade();
    let preferences = preferences.clone();
    Rc::new(move || {
        let (Some(button), Some(root)) = (button.upgrade(), root.upgrade()) else {
            return;
        };
        if launcher.is_visible() {
            launcher.dialog().hide();
            return;
        }
        let dialog = launcher.dialog();
        let roots = super::super::devices::global_search_roots();
        button.add_css_class("active");
        root.set_blurred(true);
        dialog.show(roots, preferences.sort_preferences().show_hidden);
    })
}

fn folder_jump_handler(
    launcher: Rc<SearchLauncher>,
    content: &WindowContent,
    history: Rc<NavigationHistory>,
) -> Rc<dyn Fn()> {
    let button = content.header.search.downgrade();
    let root = content.blurred_root.downgrade();
    Rc::new(move || {
        let (Some(button), Some(root)) = (button.upgrade(), root.upgrade()) else {
            return;
        };
        if launcher.is_visible() {
            launcher.dialog().hide();
            return;
        }
        let dialog = launcher.dialog();
        button.remove_css_class("active");
        root.set_blurred(true);
        dialog.show_history(history.clone());
    })
}

fn activate_result(
    browser: &BrowserView,
    preview: &PreviewDrawer,
    preferences: &PreferenceManager,
    item: SearchItem,
) {
    let controller = browser.browser();
    let location = Location::local(item.path.clone());
    if item.is_directory {
        preview.clear_target();
        controller.navigate(location);
        return;
    }
    // The drawer turns on before the reveal, so the revealed selection re-targets it.
    if preferences.search_open_files_directly() {
        if let Some(hit) = item.content_hit.clone() {
            let controller = Rc::downgrade(&controller);
            let fallback = location.clone();
            gtk::glib::MainContext::default().spawn_local(async move {
                let opened = gtk::gio::spawn_blocking(move || {
                    crate::services::smart_search::external::open(&hit)
                })
                .await
                .unwrap_or(false);
                if !opened && let Some(controller) = controller.upgrade() {
                    controller.open_location(fallback);
                }
            });
        } else {
            controller.open_location(location.clone());
        }
    } else if let Some(hit) = item.content_hit.clone() {
        preview.show_hit(hit, controller.active_depth());
    } else {
        preview.show(
            FileEntry {
                location: location.clone(),
                native_name: item.path.file_name().unwrap_or_default().to_os_string(),
                thumbnail_path: None,
                display_name: item.name,
                kind: EntryKind::File,
                size: MetadataValue::Unknown,
                modified_unix_seconds: MetadataValue::Unknown,
                recent_unix_seconds: MetadataValue::Unknown,
                is_hidden: false,
                mode: MetadataValue::Unknown,
                image_dimensions: MetadataValue::Unknown,
                child_count: MetadataValue::Unknown,
                duration_seconds: MetadataValue::Unknown,
                recent_uri: None,
            },
            controller.active_depth(),
        );
    }
    browser.reveal_location(location);
}
