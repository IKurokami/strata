// SPDX-License-Identifier: MIT

use super::*;

impl PreviewDrawer {
    pub fn is_enabled(&self) -> bool {
        self.state.is_enabled()
    }

    /// Whether the full sidebar fits, by the rule the preview applies each frame.
    pub(in crate::ui) fn sidebar_fit(
        &self,
        content: &gtk::Paned,
    ) -> Option<crate::ui::window::SidebarFit> {
        self.state.sidebar_fit(content)
    }

    pub(in crate::ui) fn action(&self) -> gio::SimpleAction {
        self.state.enabled_action.clone()
    }

    pub(in crate::ui) fn clear_target(&self) {
        self.state.clear_target();
    }
}

impl PreviewState {
    pub(super) fn is_enabled(&self) -> bool {
        self.enabled.get()
    }

    pub(super) fn set_enabled(&self, enabled: bool) -> bool {
        let previous = self.enabled.replace(enabled);
        self.refresh_panel_action();
        previous
    }

    pub(super) fn refresh_panel_action(&self) {
        let visible =
            self.is_enabled() && self.revealer.reveals_child() && self.current.borrow().is_some();
        self.enabled_action.set_state(&visible.to_variant());
    }

    pub(super) fn toggle_panel(self: &Rc<Self>, entry: Option<FileEntry>, depth: Option<usize>) {
        self.toggle(entry, depth);
    }

    pub(super) fn toggle(self: &Rc<Self>, entry: Option<FileEntry>, depth: Option<usize>) {
        if self.revealer.reveals_child() {
            self.close();
        } else if let Some(entry) = preview_target(entry) {
            self.dismissed.set(false);
            self.focus_archive_on_ready.set(true);
            self.show(entry, depth);
        } else {
            self.close();
        }
    }

    pub(super) fn clear_target(self: &Rc<Self>) {
        self.cancel_pending_show();
        if self.animating.get() && self.current.borrow().is_none() {
            if self
                .split
                .borrow()
                .as_ref()
                .is_none_or(|split| !split.is_mapped())
            {
                self.hide_panel();
            }
            return;
        }
        let restore_focus = self.hide_intent();
        let frame = self.capture_closing_frame();
        self.reset_target();
        self.close_panel(frame, restore_focus);
        self.sync_sidebar_without_preview();
    }

    pub(super) fn reset_target(&self) {
        self.cancel_pending_show();
        self.reset_find();
        self.continue_playback.take();
        self.claim_on_resume.set(false);
        self.focus_archive_on_ready.set(false);
        self.animating.set(false);
        self.sizing.close();
        self.animation_generation
            .set(self.animation_generation.get().saturating_add(1));
        self.current_request.set(None);
        self.current_depth.set(None);
        self.current.borrow_mut().take();
        self.load.borrow_mut().take();
        self.clear_raw_details();
        self.cancel_loading();
        self.pdf_loads.borrow_mut().clear();
        self.clear_content();
    }
}
