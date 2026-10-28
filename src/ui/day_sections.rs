// SPDX-License-Identifier: MIT

use crate::{app::Browser, model::MetadataValue};
use gtk::{gio, glib, prelude::*, subclass::prelude::*};
use std::{cell::RefCell, rc::Rc};

type Resolve = Rc<dyn Fn(u32) -> Option<(i64, bool)>>;

mod imp {
    use super::*;
    #[derive(Default)]
    pub struct DaySections {
        pub model: RefCell<Option<gio::ListModel>>,
        pub resolve: RefCell<Option<Resolve>>,
    }
    #[glib::object_subclass]
    impl ObjectSubclass for DaySections {
        const NAME: &'static str = "StrataDaySections";
        type Type = super::DaySections;
        type Interfaces = (gio::ListModel, gtk::SectionModel);
    }
    impl ObjectImpl for DaySections {}
    impl gio::subclass::prelude::ListModelImpl for DaySections {
        fn item_type(&self) -> glib::Type {
            gtk::StringObject::static_type()
        }
        fn n_items(&self) -> u32 {
            self.model
                .borrow()
                .as_ref()
                .map_or(0, gio::ListModel::n_items)
        }
        fn item(&self, position: u32) -> Option<glib::Object> {
            self.model.borrow().as_ref()?.item(position)
        }
    }
    impl SectionModelImpl for DaySections {
        fn section(&self, position: u32) -> (u32, u32) {
            let model = self.obj();
            let count = model.n_items();
            if position >= count {
                return (count, u32::MAX);
            }
            if model
                .imp()
                .resolve
                .borrow()
                .as_ref()
                .and_then(|resolve| resolve(position))
                .is_none()
            {
                return model
                    .source()
                    .and_downcast::<gtk::SectionModel>()
                    .map_or((0, count), |source| source.section(position));
            }
            let key = model.key(position);
            let mut low = 0;
            let mut high = position;
            while low < high {
                let mid = low + (high - low) / 2;
                if model.key(mid) == key {
                    high = mid;
                } else {
                    low = mid + 1;
                }
            }
            let start = low;
            low = position + 1;
            high = count;
            while low < high {
                let mid = low + (high - low) / 2;
                if model.key(mid) == key {
                    low = mid + 1;
                } else {
                    high = mid;
                }
            }
            (start, low)
        }
    }
}

glib::wrapper! {
    pub struct DaySections(ObjectSubclass<imp::DaySections>) @implements gio::ListModel, gtk::SectionModel;
}

impl DaySections {
    pub fn source(&self) -> Option<gio::ListModel> {
        self.imp().model.borrow().clone()
    }
    pub fn new(model: &impl IsA<gio::ListModel>, resolve: Resolve) -> Self {
        let this: Self = glib::Object::new();
        this.imp().model.replace(Some(model.clone().upcast()));
        this.imp().resolve.replace(Some(resolve));
        let weak = this.downgrade();
        model.connect_items_changed(move |_, position, removed, added| {
            if let Some(this) = weak.upgrade() {
                this.items_changed(position, removed, added);
                if this.n_items() > 0 {
                    this.sections_changed(0, this.n_items());
                }
            }
        });
        this
    }

    fn key(&self, position: u32) -> Option<(i32, i32, bool)> {
        let (timestamp, folder) = self.imp().resolve.borrow().as_ref()?(position)?;
        Some(
            glib::DateTime::from_unix_local(timestamp).map_or((i32::MIN, 0, folder), |date| {
                (date.year(), date.day_of_year(), folder)
            }),
        )
    }

    pub fn header_factory(&self) -> gtk::SignalListItemFactory {
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(|_, object| {
            let Some(header) = object.downcast_ref::<gtk::ListHeader>() else {
                return;
            };
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            row.add_css_class("day-group-heading");
            let label = gtk::Label::new(None);
            label.set_xalign(0.0);
            row.append(&label);
            let line = gtk::Separator::new(gtk::Orientation::Horizontal);
            line.set_hexpand(true);
            line.set_valign(gtk::Align::Center);
            row.append(&line);
            header.set_child(Some(&row));
        });
        let weak = self.downgrade();
        factory.connect_bind(move |_, object| {
            let Some(header) = object.downcast_ref::<gtk::ListHeader>() else {
                return;
            };
            let Some(row) = header.child() else { return };
            let timestamp = weak.upgrade().and_then(|model| {
                model
                    .imp()
                    .resolve
                    .borrow()
                    .as_ref()
                    .and_then(|resolve| resolve(header.start()))
                    .map(|value| value.0)
            });
            let date =
                timestamp.and_then(|timestamp| glib::DateTime::from_unix_local(timestamp).ok());
            let fallback = weak
                .upgrade()
                .and_then(|model| model.source())
                .and_downcast::<gtk::SortListModel>()
                .filter(|model| model.section_sorter().is_some())
                .and_then(|_| header.item().and_downcast::<gtk::StringObject>())
                .map(|item| crate::ui::browser::model_type_group(&item.string()));
            row.set_visible(timestamp.is_some() || fallback.is_some());
            if let Some(label) = row.first_child().and_downcast::<gtk::Label>() {
                label.set_text(&if let Some(fallback) = fallback {
                    crate::i18n::tr(&fallback)
                } else {
                    date.and_then(|date| date.format("%x").ok())
                        .map_or_else(|| crate::i18n::tr("Unknown date"), |date| date.to_string())
                });
            }
        });
        factory
    }
}

pub(super) fn timestamp(browser: &Browser, depth: usize, position: usize) -> Option<(i64, bool)> {
    let preferences = browser.column_preferences(depth)?;
    if preferences.sort_key != crate::model::SortKey::Modified {
        return None;
    }
    browser
        .with_entries(depth, position..position.saturating_add(1), |entries| {
            let entry = entries.first()?;
            let timestamp = match entry.modified_unix_seconds {
                MetadataValue::Known(value) => value,
                _ => i64::MIN,
            };
            Some((timestamp, preferences.folders_first && entry.is_directory()))
        })
        .flatten()
}
