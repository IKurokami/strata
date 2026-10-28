// SPDX-License-Identifier: MIT

use super::*;
use crate::services::smart_search::result::ContentLocator;

impl PreviewState {
    pub(super) fn install_search_interaction_guard(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
        scroll.set_propagation_phase(gtk::PropagationPhase::Capture);
        scroll.connect_scroll(move |_, _, _| {
            if let Some(state) = weak.upgrade() {
                state.cancel_search_reveal();
            }
            glib::Propagation::Proceed
        });
        self.pane.add_controller(scroll);
        let weak = Rc::downgrade(self);
        let click = gtk::GestureClick::new();
        click.set_button(0);
        click.set_propagation_phase(gtk::PropagationPhase::Capture);
        click.connect_pressed(move |_, _, _, _| {
            if let Some(state) = weak.upgrade() {
                state.cancel_search_reveal();
            }
        });
        self.pane.add_controller(click);
        let weak = Rc::downgrade(self);
        let controller = gtk::EventControllerKey::new();
        controller.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys::on_press(&controller, move |_, _| {
            if let Some(state) = weak.upgrade() {
                state.cancel_search_reveal();
            }
            glib::Propagation::Proceed
        });
        self.pane.add_controller(controller);
    }

    pub(super) fn cancel_search_reveal(&self) {
        crate::services::smart_search::note_interaction();
        self.interaction_epoch
            .set(self.interaction_epoch.get().wrapping_add(1));
        if let Some(state) = self.source_preview.virtual_state.borrow().as_ref() {
            state.cancel_pending_reveal();
        }
        if let Some(state) = self.rendered_text.borrow().as_ref() {
            state.cancel_pending_reveal();
        }
        if let Some(preview) = self.document_preview.borrow().as_ref()
            && let Some(state) = preview.rendered_state.borrow().as_ref()
        {
            state.cancel_pending_reveal();
        }
    }

    pub(super) fn apply_search_locator(self: &Rc<Self>) {
        let hit = self.search_hit.borrow().clone();
        let range = hit.as_ref().and_then(|hit| hit.locator.time_range());
        if let Some(audio) = self.audio.borrow().as_ref() {
            audio.view.set_match_range(range);
        }
        if let Some(video) = self.video.borrow().as_ref() {
            video.view.set_match_range(range);
        }
        let Some(hit) = hit else { return };
        if let Some((start, end)) = range {
            self.content.append(&document_notice(&rust_i18n::t!(
                "Approximate match: %{start}–%{end}",
                start = audio::clock(start.min(i64::MAX as u64) as i64),
                end = audio::clock(end.min(i64::MAX as u64) as i64)
            )));
        }
        let request = self.current_request.get();
        let epoch = self.interaction_epoch.get();
        let weak = Rc::downgrade(self);
        let attempts = Cell::new(0);
        glib::timeout_add_local(Duration::from_millis(16), move || {
            let Some(state) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            attempts.set(attempts.get() + 1);
            if state.current_request.get() != request
                || state.interaction_epoch.get() != epoch
                || state
                    .search_hit
                    .borrow()
                    .as_ref()
                    .is_none_or(|current| !Arc::ptr_eq(current, &hit))
                || attempts.get() > 120
            {
                return glib::ControlFlow::Break;
            }
            match &hit.locator {
                ContentLocator::Audio { start_us, .. } | ContentLocator::Video { start_us, .. } => {
                    let Some(media) = state.media.borrow().clone() else {
                        return glib::ControlFlow::Continue;
                    };
                    if !media.is_prepared() || !media.is_seekable() || media.duration() <= 0 {
                        return glib::ControlFlow::Continue;
                    }
                    if let Some(video) = state.video.borrow().as_ref() {
                        video.view.seek_to_match(*start_us);
                    }
                    media.seek((*start_us).min(media_last_frame(media.duration()) as u64) as i64);
                }
                ContentLocator::Text {
                    start_char,
                    end_char,
                    anchor,
                    ..
                } => {
                    let rendered = state
                        .document_preview
                        .borrow()
                        .as_ref()
                        .and_then(|preview| preview.rendered_state.borrow().clone())
                        .or_else(|| state.rendered_text.borrow().clone());
                    if let Some(rendered) = rendered {
                        if let Some(found) = rendered.find_matches(anchor).into_iter().next() {
                            rendered.select_match(found);
                            return glib::ControlFlow::Break;
                        }
                        if state.document_preview.borrow().is_some() {
                            state.show_document_view(DocumentView::Source);
                            return glib::ControlFlow::Continue;
                        }
                        state.content.append(&document_notice(&crate::i18n::tr("Match location is shown in the search excerpt; this viewer cannot jump to it")));
                        return glib::ControlFlow::Break;
                    }
                    if let Some(virtualized) = state.source_preview.virtual_state.borrow().clone() {
                        virtualized
                            .reveal_character_range(*start_char as usize, *end_char as usize);
                    } else if let Some(view) = state.source_preview.view.borrow().clone() {
                        let buffer = view.buffer();
                        if buffer.char_count() < *end_char as i32 {
                            return glib::ControlFlow::Continue;
                        }
                        let start = buffer.iter_at_offset(*start_char as i32);
                        let end = buffer.iter_at_offset(*end_char as i32);
                        buffer.select_range(&start, &end);
                        let mut start = start;
                        view.scroll_to_iter(&mut start, 0.15, false, 0.0, 0.0);
                    } else {
                        return glib::ControlFlow::Continue;
                    }
                }
                ContentLocator::Code { line_1, anchor, .. } => {
                    if let Some(view) = state.source_preview.view.borrow().clone() {
                        let buffer = view.buffer();
                        if buffer.line_count() < *line_1 as i32 {
                            return glib::ControlFlow::Continue;
                        }
                        if let Some(mut line) = buffer.iter_at_line(line_1.saturating_sub(1) as i32)
                        {
                            let mut end = line;
                            end.forward_chars(anchor.chars().count().min(i32::MAX as usize) as i32);
                            buffer.select_range(&line, &end);
                            view.scroll_to_iter(&mut line, 0.15, false, 0.0, 0.0);
                        }
                    } else if let Some(virtualized) =
                        state.source_preview.virtual_state.borrow().clone()
                    {
                        if let Some(found) = virtualized.find_matches(anchor).into_iter().next() {
                            virtualized.select_match(found);
                        }
                    } else {
                        return glib::ControlFlow::Continue;
                    }
                }
                ContentLocator::Slide { .. } | ContentLocator::Sheet { .. } => {
                    state.content.append(&document_notice(&crate::i18n::tr("Match location is shown in the search excerpt; this viewer cannot jump to it")));
                }
                _ => {}
            }
            glib::ControlFlow::Break
        });
    }
}
