// SPDX-License-Identifier: MIT

use gtk::{glib, prelude::*, subclass::prelude::*};
use std::{
    cell::{Cell, OnceCell, RefCell},
    sync::OnceLock,
};

mod canvas {
    use super::*;
    #[derive(Default)]
    pub struct Canvas {
        pub nodes: RefCell<Vec<Node>>,
    }
    #[glib::object_subclass]
    impl ObjectSubclass for Canvas {
        const NAME: &'static str = "StrataUsageCanvas";
        type Type = super::Canvas;
        type ParentType = gtk::DrawingArea;
        type Interfaces = (gtk::Accessible,);
    }
    impl ObjectImpl for Canvas {
        fn dispose(&self) {
            self.nodes.borrow_mut().clear();
        }
    }
    impl WidgetImpl for Canvas {}
    impl DrawingAreaImpl for Canvas {}
    impl AccessibleImpl for Canvas {
        fn first_accessible_child(&self) -> Option<gtk::Accessible> {
            self.nodes
                .borrow()
                .first()
                .map(|node| node.clone().upcast())
        }
    }
}

glib::wrapper! {
    pub struct Canvas(ObjectSubclass<canvas::Canvas>)
        @extends gtk::DrawingArea, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

mod node {
    use super::*;
    #[derive(Default)]
    pub struct Node {
        pub parent: glib::WeakRef<super::Canvas>,
        pub next: glib::WeakRef<super::Node>,
        pub context: OnceCell<Option<gtk::ATContext>>,
        pub bounds: Cell<Option<(i32, i32, i32, i32)>>,
    }
    #[glib::object_subclass]
    impl ObjectSubclass for Node {
        const NAME: &'static str = "StrataUsageAccessibleItem";
        type Type = super::Node;
        type Interfaces = (gtk::Accessible,);
    }
    impl ObjectImpl for Node {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: OnceLock<Vec<glib::ParamSpec>> = OnceLock::new();
            PROPERTIES.get_or_init(|| {
                vec![
                    glib::ParamSpecEnum::builder_with_default(
                        "accessible-role",
                        gtk::AccessibleRole::ListItem,
                    )
                    .read_only()
                    .build(),
                ]
            })
        }
        fn property(&self, _: usize, _: &glib::ParamSpec) -> glib::Value {
            gtk::AccessibleRole::ListItem.to_value()
        }
    }
    impl AccessibleImpl for Node {
        fn at_context(&self) -> Option<gtk::ATContext> {
            self.context
                .get_or_init(|| {
                    let parent = self.parent.upgrade()?;
                    gtk::ATContext::create(
                        gtk::AccessibleRole::ListItem,
                        &*self.obj(),
                        &parent.display(),
                    )
                })
                .clone()
        }
        fn accessible_parent(&self) -> Option<gtk::Accessible> {
            self.parent.upgrade().map(|parent| parent.upcast())
        }
        fn next_accessible_sibling(&self) -> Option<gtk::Accessible> {
            self.next.upgrade().map(|node| node.upcast())
        }
        fn first_accessible_child(&self) -> Option<gtk::Accessible> {
            None
        }
        fn platform_state(&self, _: gtk::AccessiblePlatformState) -> bool {
            false
        }
        fn bounds(&self) -> Option<(i32, i32, i32, i32)> {
            self.bounds.get()
        }
    }
}

glib::wrapper! { pub struct Node(ObjectSubclass<node::Node>) @implements gtk::Accessible; }

impl Canvas {
    pub fn new() -> Self {
        glib::Object::builder()
            .property("hexpand", true)
            .property("vexpand", true)
            .property("focusable", true)
            .property("accessible-role", gtk::AccessibleRole::ListBox)
            .build()
    }

    pub fn set_items(&self, labels: &[String]) {
        for node in self.imp().nodes.take() {
            node.imp().parent.set(None);
            node.imp().next.set(None);
            node.set_accessible_parent(gtk::Accessible::NONE, gtk::Accessible::NONE);
        }
        let nodes: Vec<Node> = labels
            .iter()
            .map(|label| {
                let node: Node = glib::Object::new();
                node.imp().parent.set(Some(self));
                node.update_property(&[gtk::accessible::Property::Label(label)]);
                node
            })
            .collect();
        for (index, node) in nodes.iter().enumerate() {
            node.imp().next.set(nodes.get(index + 1));
            node.set_accessible_parent(Some(self), nodes.get(index + 1));
        }
        self.imp().nodes.replace(nodes);
    }

    pub fn set_bounds(&self, rectangles: &[super::layout::Rect]) {
        for (node, rect) in self.imp().nodes.borrow().iter().zip(rectangles) {
            node.imp().bounds.set(Some((
                rect.x as i32,
                rect.y as i32,
                rect.width as i32,
                rect.height as i32,
            )));
        }
    }

    pub fn set_selected(&self, position: Option<usize>) {
        for (index, node) in self.imp().nodes.borrow().iter().enumerate() {
            let selected = position == Some(index);
            node.update_state(&[gtk::accessible::State::Selected(Some(selected))]);
            if selected {
                self.update_relation(&[gtk::accessible::Relation::ActiveDescendant(
                    node.upcast_ref(),
                )]);
            }
        }
        if position.is_none() {
            self.reset_relation(gtk::AccessibleRelation::ActiveDescendant);
        }
    }
}
