// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    rc::Rc,
};

use gio::subclass::prelude::ListModelImpl;
use gtk::{gio, glib, prelude::*, subclass::prelude::*};

pub(crate) type ResolveFn = Rc<dyn Fn(u32) -> Option<String>>;
pub(crate) type IsHiddenFn = Rc<dyn Fn(u32) -> Option<bool>>;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct EntryListModel {
        pub count: Cell<u32>,
        pub resolve: RefCell<Option<ResolveFn>>,
        pub is_hidden: RefCell<Option<IsHiddenFn>>,
        pub objects: RefCell<BTreeMap<u32, glib::WeakRef<gtk::StringObject>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for EntryListModel {
        const NAME: &'static str = "StrataEntryListModel";
        type Type = super::EntryListModel;
        type Interfaces = (gio::ListModel,);
    }

    impl ObjectImpl for EntryListModel {}

    impl ListModelImpl for EntryListModel {
        fn item_type(&self) -> glib::Type {
            gtk::StringObject::static_type()
        }

        fn n_items(&self) -> u32 {
            self.count.get()
        }

        fn item(&self, position: u32) -> Option<glib::Object> {
            if position >= self.count.get() {
                return None;
            }
            if let Some(object) = self
                .objects
                .borrow()
                .get(&position)
                .and_then(glib::WeakRef::upgrade)
            {
                return Some(object.upcast());
            }
            let resolve = self.resolve.borrow().clone()?;
            let object = gtk::StringObject::new(&resolve(position)?);
            self.objects
                .borrow_mut()
                .insert(position, object.downgrade());
            Some(object.upcast())
        }
    }
}

glib::wrapper! {
    pub struct EntryListModel(ObjectSubclass<imp::EntryListModel>)
        @implements gio::ListModel;
}

impl EntryListModel {
    pub(crate) fn new(resolve: ResolveFn) -> Self {
        let model: Self = glib::Object::new();
        model.imp().resolve.replace(Some(resolve));
        model
    }

    pub(crate) fn set_is_hidden(&self, is_hidden: IsHiddenFn) {
        self.imp().is_hidden.replace(Some(is_hidden));
    }

    pub(crate) fn is_hidden(&self, position: u32) -> Option<bool> {
        if position >= self.imp().count.get() {
            return None;
        }
        let is_hidden = self.imp().is_hidden.borrow().clone()?;
        is_hidden(position)
    }

    pub(crate) fn value(&self, position: u32) -> Option<String> {
        if position >= self.imp().count.get() {
            return None;
        }
        let resolve = self.imp().resolve.borrow().clone()?;
        resolve(position)
    }

    pub(crate) fn replace(&self, count: u32) {
        self.imp().objects.borrow_mut().clear();
        let removed = self.imp().count.replace(count);
        // The authoritative entries may have been reordered or replaced
        // without changing their count, so every non-empty replacement must
        // invalidate the corresponding GTK items.
        if removed > 0 || count > 0 {
            self.items_changed(0, removed, count);
        }
    }

    pub(crate) fn splice(&self, position: u32, removed: u32, added: u32) {
        let count = self.imp().count.get();
        let position = position.min(count);
        let removed = removed.min(count - position);
        let new_count = (count - removed).saturating_add(added);
        if removed > 0 || added > 0 {
            let mut objects = self.imp().objects.borrow_mut();
            let tail = objects.split_off(&position);
            for (index, object) in tail {
                if index >= position + removed
                    && object.upgrade().is_some()
                    && let Some(index) = (index - removed)
                        .checked_add(added)
                        .filter(|index| *index < new_count)
                {
                    objects.insert(index, object);
                }
            }
        }
        self.imp().count.set(new_count);
        if removed > 0 || added > 0 {
            self.items_changed(position, removed, added);
        }
    }
}

#[cfg(test)]
mod tests;
