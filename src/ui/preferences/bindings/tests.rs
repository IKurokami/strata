// SPDX-License-Identifier: MIT

use super::*;
use crate::test_support::gtk_test;

fn read_saved_preferences() -> io::Result<Preferences> {
    flush_pending_preferences();
    read_preferences()
}

#[test]
fn rapid_changes_apply_live_before_the_latest_snapshot_is_persisted() {
    gtk_test(
        "ui::preferences::bindings::tests::rapid_changes_apply_live_before_the_latest_snapshot_is_persisted",
        || {
            let manager = PreferenceManager::shared();
            let anchors = [
                gtk::Box::new(gtk::Orientation::Vertical, 0),
                gtk::Box::new(gtk::Orientation::Vertical, 0),
            ];
            let values = anchors.each_ref().map(|anchor| {
                let value = Rc::new(Cell::new(0.0));
                let observed = value.clone();
                manager.bind_preference(
                    anchor,
                    PreferenceManager::preview_volume,
                    move |_, volume| observed.set(volume),
                );
                value
            });
            for volume in [0.2, 0.4, 0.8, 0.35] {
                manager.set_preview_volume(volume);
                for value in &values {
                    assert_eq!(value.get(), volume);
                }
            }
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while manager.persistence_dirty.get() {
                assert!(
                    std::time::Instant::now() < deadline,
                    "background save did not complete"
                );
                glib::MainContext::default().iteration(false);
            }
            assert_eq!(
                read_preferences()
                    .expect("background settings")
                    .preview_volume,
                0.35
            );
        },
    );
}

#[test]
fn bindings_initialize_deduplicate_and_release_destroyed_anchors() {
    gtk_test(
        "ui::preferences::bindings::tests::bindings_initialize_deduplicate_and_release_destroyed_anchors",
        || {
            let manager = PreferenceManager::shared();
            let anchor = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let values = Rc::new(RefCell::new(Vec::new()));
            let observed = values.clone();
            manager.bind_preference(
                &anchor,
                PreferenceManager::folder_peeking,
                move |_, value| observed.borrow_mut().push(value),
            );
            assert_eq!(*values.borrow(), [false]);
            manager.set_folder_peeking(true);
            manager.set_folder_peeking(true);
            manager.set_type_to_search(false);
            assert_eq!(*values.borrow(), [false, true]);
            let revision = manager.changes.revision.get();
            manager.set_type_to_search(false);
            assert_eq!(manager.changes.revision.get(), revision);
            drop(anchor);
            assert!(manager.changes.listeners.borrow().is_empty());
            assert_eq!(Rc::strong_count(&values), 1);
        },
    );
}

#[test]
fn destroying_an_anchor_can_release_another_bound_anchor() {
    gtk_test(
        "ui::preferences::bindings::tests::destroying_an_anchor_can_release_another_bound_anchor",
        || {
            let manager = PreferenceManager::shared();
            let owner = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let dependent = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let dependent_weak = dependent.downgrade();
            let calls = Rc::new(Cell::new(0));
            let observed = calls.clone();
            manager.bind_preference(
                &dependent,
                PreferenceManager::folder_peeking,
                move |_, _| {
                    observed.set(observed.get() + 1);
                },
            );
            manager.bind_preference(
                &owner,
                PreferenceManager::folder_peeking,
                move |_, value| {
                    dependent.set_visible(value);
                },
            );
            assert_eq!(calls.get(), 1);
            drop(owner);
            assert!(dependent_weak.upgrade().is_none());
            assert_eq!(manager.listener_count(), 0);
            manager.set_folder_peeking(true);
            assert_eq!(
                calls.get(),
                1,
                "destroyed bindings must not be called again"
            );
        },
    );
}

#[test]
fn failed_saves_still_apply_and_retry_without_repeating_notifications() {
    gtk_test(
        "ui::preferences::bindings::tests::failed_saves_still_apply_and_retry_without_repeating_notifications",
        || {
            let manager = PreferenceManager::shared();
            let anchor = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let values = Rc::new(RefCell::new(Vec::new()));
            let observed = values.clone();
            manager.bind_preference(
                &anchor,
                PreferenceManager::folder_peeking,
                move |_, value| observed.borrow_mut().push(value),
            );
            fs::create_dir_all(settings_path()).expect("block settings file with a directory");
            manager.set_folder_peeking(true);
            assert_eq!(*values.borrow(), [false, true]);
            manager.flush_preferences();
            assert!(manager.persistence_dirty.get());
            fs::remove_dir(settings_path()).expect("remove write failure fixture");
            manager.set_folder_peeking(true);
            manager.flush_preferences();
            assert!(!manager.persistence_dirty.get());
            assert!(
                read_saved_preferences()
                    .expect("retried save")
                    .folder_peeking
            );
            assert_eq!(*values.borrow(), [false, true]);
        },
    );
}

#[test]
fn reentrant_changes_reach_all_bindings_without_notification_loops() {
    gtk_test(
        "ui::preferences::bindings::tests::reentrant_changes_reach_all_bindings_without_notification_loops",
        || {
            let manager = PreferenceManager::shared();
            let anchor = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let values = Rc::new(RefCell::new(Vec::new()));
            let observed = values.clone();
            manager.bind_preference(
                &anchor,
                PreferenceManager::type_to_search,
                move |_, value| observed.borrow_mut().push(value),
            );
            let weak = Rc::downgrade(&manager);
            manager.bind_preference(
                &anchor,
                PreferenceManager::folder_peeking,
                move |_, value| {
                    if let Some(manager) = weak.upgrade() {
                        manager.set_folder_peeking(value);
                        manager.set_type_to_search(value);
                    }
                },
            );
            manager.set_folder_peeking(false);
            assert_eq!(*values.borrow(), [true, false]);
            manager.set_folder_peeking(true);
            assert_eq!(*values.borrow(), [true, false, true]);
        },
    );
}

#[test]
fn a_channel_change_reaches_only_the_views_that_still_exist() {
    let ran = RefCell::new(Vec::new());
    let listeners = RefCell::new(vec![(1, true), (2, false), (3, true)]);
    notify_live(
        &listeners,
        |(_, alive)| *alive,
        |(id, _)| ran.borrow_mut().push(*id),
    );

    assert_eq!(ran.into_inner(), vec![1, 3]);
    assert_eq!(listeners.into_inner(), vec![(1, true), (3, true)]);
}

#[test]
fn a_channel_change_with_no_surviving_views_clears_the_registry() {
    let ran = RefCell::new(0_u32);
    let listeners = RefCell::new(vec![(1, false)]);
    notify_live(&listeners, |(_, alive)| *alive, |_| *ran.borrow_mut() += 1);

    assert_eq!(ran.into_inner(), 0);
    assert!(listeners.into_inner().is_empty());
}

#[test]
fn listeners_added_during_notification_remain_registered_for_the_next_change() {
    let listeners = RefCell::new(vec![(1, true)]);
    notify_live(
        &listeners,
        |(_, alive)| *alive,
        |_| listeners.borrow_mut().push((2, true)),
    );

    assert_eq!(listeners.into_inner(), vec![(1, true), (2, true)]);
}
