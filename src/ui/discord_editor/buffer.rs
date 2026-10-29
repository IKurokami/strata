// SPDX-License-Identifier: MIT

use super::model::{Document, Style};
use gtk::prelude::*;
use std::{cell::RefCell, collections::HashMap};

#[derive(Default)]
pub(super) struct RichBuffer {
    pub buffer: gtk::TextBuffer,
    tags: RefCell<HashMap<Style, gtk::TextTag>>,
    styles: RefCell<HashMap<gtk::TextTag, Style>>,
}

impl RichBuffer {
    pub fn document(&self) -> Document {
        let mut document = Document::default();
        let mut start = self.buffer.start_iter();
        let finish = self.buffer.end_iter();
        let tags = self.styles.borrow();
        while start.offset() < finish.offset() {
            let style = start
                .tags()
                .iter()
                .filter_map(|tag| tags.get(tag).map(|style| (tag.priority(), *style)))
                .max_by_key(|(priority, _)| *priority)
                .map(|(_, style)| style)
                .unwrap_or_default();
            let mut end = start;
            if !end.forward_to_tag_toggle(None::<&gtk::TextTag>) {
                end = finish;
            }
            if end.offset() <= start.offset() {
                break;
            }
            document.push(&self.buffer.text(&start, &end, true), style);
            start = end;
        }
        document
    }

    pub fn show(&self, document: &Document) {
        self.buffer.set_text("");
        self.styles.borrow_mut().clear();
        for (_, tag) in self.tags.borrow_mut().drain() {
            self.buffer.tag_table().remove(&tag);
        }
        let mut position = 0;
        self.buffer.set_text(&document.text());
        for run in &document.runs {
            let length = run.text.chars().count() as i32;
            let tag = self
                .tags
                .borrow_mut()
                .entry(run.style)
                .or_insert_with(|| {
                    let style = run.style;
                    let tag = gtk::TextTag::builder()
                        .weight(if style.bold { 700 } else { 400 })
                        .style(if style.italic {
                            gtk::pango::Style::Italic
                        } else {
                            gtk::pango::Style::Normal
                        })
                        .underline(if style.underline {
                            gtk::pango::Underline::Single
                        } else {
                            gtk::pango::Underline::None
                        })
                        .strikethrough(style.strike)
                        .build();
                    if let Some(color) = style.foreground {
                        tag.set_foreground(Some(&color.hex()));
                    }
                    if let Some(color) = style.background {
                        tag.set_background(Some(&color.hex()));
                    }
                    self.buffer.tag_table().add(&tag);
                    self.styles.borrow_mut().insert(tag.clone(), style);
                    tag
                })
                .clone();
            self.buffer.apply_tag(
                &tag,
                &self.buffer.iter_at_offset(position),
                &self.buffer.iter_at_offset(position + length),
            );
            position += length;
        }
    }
}
