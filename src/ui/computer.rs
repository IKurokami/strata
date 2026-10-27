// SPDX-License-Identifier: MIT

use crate::{
    adapters::computer::{self, DriveDetails},
    app::Browser,
    model::Location,
};
use gtk::{gio, glib, prelude::*};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
use strata_win_shell::volume::{DriveType, VolumeInfo};

pub(super) struct ComputerView {
    pub widget: gtk::ScrolledWindow,
    view: gtk::ColumnView,
    rows: gio::ListStore,
    generation: Cell<u64>,
    task: RefCell<Option<glib::JoinHandle<()>>>,
}

#[derive(Clone)]
struct DriveRow {
    volume: VolumeInfo,
    details: Option<DriveDetails>,
}

impl ComputerView {
    pub fn new(browser: &Rc<Browser>) -> Rc<Self> {
        let rows = gio::ListStore::new::<glib::BoxedAnyObject>();
        let selection = gtk::SingleSelection::new(Some(rows.clone()));
        let view = gtk::ColumnView::new(Some(selection));
        view.add_css_class("computer-table");
        view.set_show_column_separators(false);
        view.set_show_row_separators(true);
        for (index, title) in [
            "Name",
            "Type",
            "File system",
            "Free space",
            "Size",
            "Percent full",
        ]
        .iter()
        .enumerate()
        {
            let factory = gtk::SignalListItemFactory::new();
            factory.connect_setup(move |_, object| {
                let Some(item) = object.downcast_ref::<gtk::ListItem>() else {
                    return;
                };
                if index == 5 {
                    let bar = gtk::ProgressBar::new();
                    bar.set_show_text(true);
                    bar.set_hexpand(true);
                    bar.add_css_class("drive-capacity");
                    item.set_child(Some(&bar));
                } else {
                    let label = gtk::Label::new(None);
                    label.set_xalign(0.0);
                    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
                    if index == 0 {
                        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                        row.append(&crate::assets::primary_icon(
                            crate::assets::icons::HARD_DRIVE,
                            20,
                        ));
                        row.append(&label);
                        item.set_child(Some(&row));
                    } else {
                        item.set_child(Some(&label));
                    }
                }
            });
            factory.connect_bind(move |_, object| {
                let Some(item) = object.downcast_ref::<gtk::ListItem>() else {
                    return;
                };
                let Some(object) = item.item().and_downcast::<glib::BoxedAnyObject>() else {
                    return;
                };
                let row = object.borrow::<DriveRow>();
                let Some(child) = item.child() else { return };
                if index == 5 {
                    let Some(bar) = child.downcast_ref::<gtk::ProgressBar>() else {
                        return;
                    };
                    let used = row
                        .details
                        .as_ref()
                        .and_then(|details| details.space)
                        .filter(|space| space.total > 0)
                        .map(|space| {
                            1.0 - space.available.min(space.total) as f64 / space.total as f64
                        });
                    bar.set_fraction(used.unwrap_or(0.0));
                    bar.set_text(Some(&used.map_or_else(
                        || "—".to_owned(),
                        |fraction| crate::i18n::percent((fraction * 100.0).round() as usize),
                    )));
                    if used.is_some_and(|used| used >= 0.9) {
                        bar.add_css_class("nearly-full");
                    } else {
                        bar.remove_css_class("nearly-full");
                    }
                } else {
                    let label = if index == 0 {
                        child.last_child().and_downcast::<gtk::Label>()
                    } else {
                        child.downcast::<gtk::Label>().ok()
                    };
                    if let Some(label) = label {
                        label.set_text(&row.text(index));
                    }
                }
            });
            let column = gtk::ColumnViewColumn::new(Some(&crate::i18n::tr(title)), Some(factory));
            column.set_resizable(true);
            column.set_fixed_width(if index == 0 {
                220
            } else if index == 5 {
                200
            } else {
                125
            });
            column.set_expand(index == 0 || index == 5);
            view.append_column(&column);
        }
        let navigate = browser.clone();
        let entries = rows.clone();
        view.connect_activate(move |_, index| {
            if let Some(object) = entries.item(index).and_downcast::<glib::BoxedAnyObject>() {
                let root = object.borrow::<DriveRow>().volume.root.clone();
                navigate.navigate_location(Location::local(root), true);
            }
        });
        let widget = gtk::ScrolledWindow::builder()
            .child(&view)
            .hexpand(true)
            .vexpand(true)
            .build();
        widget.add_css_class("computer-page");
        let this = Rc::new(Self {
            widget,
            view,
            rows,
            generation: Cell::new(0),
            task: RefCell::new(None),
        });
        let weak = Rc::downgrade(&this);
        this.widget.connect_map(move |_| {
            if let Some(this) = weak.upgrade() {
                this.focus();
            }
        });
        this
    }

    pub fn focus(&self) {
        self.view.grab_focus();
    }

    pub fn refresh(self: &Rc<Self>) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
        let generation = self.generation.get().wrapping_add(1);
        self.generation.set(generation);
        let weak = Rc::downgrade(self);
        let task = glib::MainContext::default().spawn_local(async move {
            let Ok(mut volumes) = gio::spawn_blocking(strata_win_shell::volume::drive_roots).await
            else {
                return;
            };
            // Queue local disks before devices whose providers may be offline.
            volumes.sort_by_key(|volume| {
                matches!(volume.drive_type, DriveType::Remote | DriveType::CdRom)
            });
            let Some(this) = weak
                .upgrade()
                .filter(|this| this.generation.get() == generation)
            else {
                return;
            };
            let rows: Vec<_> = volumes
                .iter()
                .map(|volume| {
                    glib::BoxedAnyObject::new(DriveRow {
                        volume: volume.clone(),
                        details: None,
                    })
                })
                .collect();
            this.rows.splice(0, this.rows.n_items(), &rows);
            drop(this);
            let mut pending = Vec::new();
            for volume in volumes {
                let weak = weak.clone();
                pending.push(async move {
                    let Some(details) = computer::details(volume).await else {
                        return;
                    };
                    let Some(this) = weak
                        .upgrade()
                        .filter(|this| this.generation.get() == generation)
                    else {
                        return;
                    };
                    for index in 0..this.rows.n_items() {
                        let Some(object) =
                            this.rows.item(index).and_downcast::<glib::BoxedAnyObject>()
                        else {
                            continue;
                        };
                        if object.borrow::<DriveRow>().volume.root == details.volume.root {
                            let row = DriveRow {
                                volume: details.volume.clone(),
                                details: Some(details),
                            };
                            this.rows
                                .splice(index, 1, &[glib::BoxedAnyObject::new(row)]);
                            break;
                        }
                    }
                });
            }
            // Poll all requests together so a slow disk cannot delay faster rows.
            let mut pending: Vec<_> = pending.into_iter().map(Box::pin).collect();
            futures_lite::future::poll_fn(move |cx| {
                pending.retain_mut(|future| {
                    std::future::Future::poll(future.as_mut(), cx).is_pending()
                });
                if pending.is_empty() {
                    std::task::Poll::Ready(())
                } else {
                    std::task::Poll::Pending
                }
            })
            .await;
        });
        *self.task.borrow_mut() = Some(task);
    }
}

impl Drop for ComputerView {
    fn drop(&mut self) {
        if let Some(task) = self.task.get_mut().take() {
            task.abort();
        }
    }
}

impl DriveRow {
    fn text(&self, index: usize) -> String {
        let volume = self
            .details
            .as_ref()
            .map_or(&self.volume, |details| &details.volume);
        match index {
            0 => volume.label.as_ref().map_or_else(
                || volume.root.display().to_string(),
                |label| format!("{label} ({})", volume.root.display()),
            ),
            1 => crate::i18n::tr(match volume.drive_type {
                DriveType::Fixed => "Local disk",
                DriveType::Removable => "Removable drive",
                DriveType::Remote => "Network drive",
                DriveType::CdRom => "Optical drive",
                DriveType::RamDisk => "RAM disk",
                _ => "Drive",
            }),
            2 => volume.fs_type.clone().unwrap_or_else(|| "—".to_owned()),
            3 | 4 => self
                .details
                .as_ref()
                .and_then(|details| details.space)
                .map_or_else(
                    || "—".to_owned(),
                    |space| {
                        crate::i18n::file_size(if index == 3 {
                            space.available
                        } else {
                            space.total
                        })
                    },
                ),
            _ => String::new(),
        }
    }
}
