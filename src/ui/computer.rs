// SPDX-License-Identifier: MIT

use crate::{
    adapters::computer::{self, DriveDetails},
    app::Browser,
    model::Location,
};
use gtk::{gio, glib, prelude::*};
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
};
use strata_win_shell::volume::{DriveType, VolumeInfo};

pub(super) struct ComputerView {
    pub widget: gtk::ScrolledWindow,
    drives: gtk::FlowBox,
    tiles: RefCell<Vec<DriveTile>>,
    generation: Cell<u64>,
    task: RefCell<Option<glib::JoinHandle<()>>>,
    _volume_monitor: gio::VolumeMonitor,
}

struct DriveTile {
    root: PathBuf,
    child: gtk::FlowBoxChild,
    name: gtk::Label,
    kind: gtk::Label,
    file_system: gtk::Label,
    bar: gtk::ProgressBar,
    capacity: gtk::Label,
    percent: gtk::Label,
}

impl DriveTile {
    fn new(volume: &VolumeInfo) -> Self {
        let icon = crate::assets::primary_icon(drive_icon(volume.drive_type), 28);
        icon.add_css_class("drive-tile-icon");
        let icon_frame = gtk::Box::new(gtk::Orientation::Vertical, 0);
        icon_frame.add_css_class("drive-tile-icon-frame");
        icon_frame.set_valign(gtk::Align::Start);
        icon_frame.append(&icon);

        let name = gtk::Label::new(None);
        name.add_css_class("drive-tile-name");
        name.set_xalign(0.0);
        name.set_hexpand(true);
        name.set_ellipsize(gtk::pango::EllipsizeMode::End);

        let file_system = gtk::Label::new(None);
        file_system.add_css_class("drive-tile-badge");
        file_system.set_valign(gtk::Align::Center);

        let title = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        title.append(&name);
        title.append(&file_system);

        let kind = gtk::Label::new(None);
        kind.add_css_class("drive-tile-kind");
        kind.set_xalign(0.0);
        kind.set_ellipsize(gtk::pango::EllipsizeMode::End);

        let bar = gtk::ProgressBar::new();
        bar.add_css_class("drive-capacity");
        bar.set_hexpand(true);
        bar.update_property(&[gtk::accessible::Property::Label(&crate::i18n::tr(
            "Percent full",
        ))]);

        let capacity = gtk::Label::new(None);
        capacity.add_css_class("drive-tile-capacity");
        capacity.set_xalign(0.0);
        capacity.set_hexpand(true);
        capacity.set_ellipsize(gtk::pango::EllipsizeMode::End);

        let percent = gtk::Label::new(None);
        percent.add_css_class("drive-tile-percent");
        percent.set_xalign(1.0);

        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        footer.append(&capacity);
        footer.append(&percent);

        let details = gtk::Box::new(gtk::Orientation::Vertical, 4);
        details.set_hexpand(true);
        details.append(&title);
        details.append(&kind);
        details.append(&bar);
        details.append(&footer);

        let content = gtk::Box::new(gtk::Orientation::Horizontal, 14);
        content.add_css_class("drive-tile");
        content.append(&icon_frame);
        content.append(&details);

        let child = gtk::FlowBoxChild::new();
        child.set_child(Some(&content));
        let tile = Self {
            root: volume.root.clone(),
            child,
            name,
            kind,
            file_system,
            bar,
            capacity,
            percent,
        };
        tile.update(volume, None);
        tile
    }

    fn update(&self, volume: &VolumeInfo, details: Option<&DriveDetails>) {
        let volume = details.map_or(volume, |details| &details.volume);
        let custom_label = crate::ui::preferences::PreferenceManager::shared()
            .device_label(&volume.root.to_string_lossy());
        let name = computer::format_drive_display_name(volume, custom_label);
        self.name.set_text(&name);
        self.child
            .update_property(&[gtk::accessible::Property::Label(&name)]);
        self.kind
            .set_text(&computer::drive_type_label(volume.drive_type));
        match volume.fs_type.as_deref().filter(|fs| !fs.is_empty()) {
            Some(fs) => {
                self.file_system.set_text(fs);
                self.file_system.set_visible(true);
            }
            None => self.file_system.set_visible(false),
        }

        let space = details
            .and_then(|details| details.space)
            .filter(|space| space.total > 0);
        match space {
            Some(space) => {
                let available = space.available.min(space.total);
                let used = 1.0 - available as f64 / space.total as f64;
                self.bar.set_fraction(used);
                self.bar.set_visible(true);
                self.capacity.set_text(&rust_i18n::t!(
                    "%{free} free of %{total}",
                    free = crate::i18n::drive_capacity(available),
                    total = crate::i18n::drive_capacity(space.total)
                ));
                self.percent
                    .set_text(&crate::i18n::percent((used * 100.0).round() as usize));
                self.percent.set_visible(true);
                if used >= 0.9 {
                    self.bar.add_css_class("nearly-full");
                } else {
                    self.bar.remove_css_class("nearly-full");
                }
            }
            None => {
                self.bar.set_fraction(0.0);
                self.bar.set_visible(details.is_none());
                self.bar.remove_css_class("nearly-full");
                self.capacity.set_text("—");
                self.percent.set_visible(false);
            }
        }
    }
}

fn drive_icon(drive_type: DriveType) -> &'static str {
    match drive_type {
        DriveType::Remote => crate::assets::icons::NETWORK,
        DriveType::CdRom => crate::assets::icons::DISC,
        _ => crate::assets::icons::HARD_DRIVE,
    }
}

impl ComputerView {
    pub fn new(browser: &Rc<Browser>) -> Rc<Self> {
        let drives = gtk::FlowBox::new();
        drives.add_css_class("computer-drives");
        drives.set_selection_mode(gtk::SelectionMode::Single);
        drives.set_activate_on_single_click(false);
        drives.set_homogeneous(true);
        drives.set_min_children_per_line(1);
        drives.set_max_children_per_line(4);
        drives.set_row_spacing(12);
        drives.set_column_spacing(12);
        drives.set_valign(gtk::Align::Start);

        let heading = gtk::Label::new(Some(&crate::i18n::tr("Devices and drives")));
        heading.add_css_class("computer-heading");
        heading.set_xalign(0.0);

        let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
        content.add_css_class("computer-content");
        content.append(&heading);
        content.append(&drives);

        let widget = gtk::ScrolledWindow::builder()
            .child(&content)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .hexpand(true)
            .vexpand(true)
            .build();
        widget.add_css_class("computer-page");
        let volume_monitor = gio::VolumeMonitor::get();
        let this = Rc::new(Self {
            widget,
            drives,
            tiles: RefCell::new(Vec::new()),
            generation: Cell::new(0),
            task: RefCell::new(None),
            _volume_monitor: volume_monitor.clone(),
        });

        let weak = Rc::downgrade(&this);
        let navigate = Rc::downgrade(browser);
        this.drives.connect_child_activated(move |_, child| {
            let (Some(this), Some(browser)) = (weak.upgrade(), navigate.upgrade()) else {
                return;
            };
            let root = this
                .tiles
                .borrow()
                .iter()
                .find(|tile| &tile.child == child)
                .map(|tile| tile.root.clone());
            if let Some(root) = root {
                browser.navigate_location(Location::local(root), true);
            }
        });

        let weak = Rc::downgrade(&this);
        this.widget.connect_map(move |_| {
            if let Some(this) = weak.upgrade() {
                this.focus();
            }
        });
        macro_rules! connect_monitor_refresh {
            ($method:ident) => {{
                let weak = Rc::downgrade(&this);
                volume_monitor.$method(move |_, _| {
                    if let Some(this) = weak.upgrade() {
                        this.refresh();
                    }
                });
            }};
        }
        connect_monitor_refresh!(connect_mount_added);
        connect_monitor_refresh!(connect_mount_removed);
        connect_monitor_refresh!(connect_mount_changed);
        connect_monitor_refresh!(connect_volume_added);
        connect_monitor_refresh!(connect_volume_removed);
        connect_monitor_refresh!(connect_volume_changed);
        connect_monitor_refresh!(connect_drive_connected);
        connect_monitor_refresh!(connect_drive_disconnected);
        connect_monitor_refresh!(connect_drive_changed);
        this
    }

    pub fn focus(&self) {
        let selected = self.drives.selected_children().into_iter().next();
        let target = selected.or_else(|| self.drives.child_at_index(0));
        match target {
            Some(child) => {
                child.grab_focus();
            }
            None => {
                self.drives.grab_focus();
            }
        }
    }

    fn replace_tiles(&self, volumes: &[VolumeInfo]) {
        let selected_root = self
            .drives
            .selected_children()
            .into_iter()
            .next()
            .and_then(|child| {
                self.tiles
                    .borrow()
                    .iter()
                    .find(|tile| tile.child == child)
                    .map(|tile| tile.root.clone())
            });
        let had_focus = self.drives.focus_child().is_some();
        self.drives.remove_all();
        let tiles: Vec<_> = volumes.iter().map(DriveTile::new).collect();
        for tile in &tiles {
            self.drives.append(&tile.child);
        }
        if let Some(tile) = selected_root
            .and_then(|root| tiles.iter().find(|tile| tile.root == root))
            .or_else(|| tiles.first())
        {
            self.drives.select_child(&tile.child);
            if had_focus {
                tile.child.grab_focus();
            }
        }
        self.tiles.replace(tiles);
    }

    fn update_tile(&self, details: &DriveDetails) {
        if let Some(tile) = self
            .tiles
            .borrow()
            .iter()
            .find(|tile| tile.root == details.volume.root)
        {
            tile.update(&details.volume, Some(details));
        }
    }

    pub fn refresh(self: &Rc<Self>) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
        let generation = self.generation.get().wrapping_add(1);
        self.generation.set(generation);
        let weak = Rc::downgrade(self);
        let task = glib::MainContext::default().spawn_local(async move {
            let Ok(mut volumes) =
                gio::spawn_blocking(strata_win_shell::volume::enumerate_volumes).await
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
            this.replace_tiles(&volumes);
            drop(this);
            let mut pending = Vec::new();
            for volume in volumes {
                let weak = weak.clone();
                pending.push(async move {
                    let Some(details) = computer::details(volume).await else {
                        return;
                    };
                    if let Some(this) = weak
                        .upgrade()
                        .filter(|this| this.generation.get() == generation)
                    {
                        this.update_tile(&details);
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
