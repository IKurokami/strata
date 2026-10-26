// SPDX-License-Identifier: MIT

use super::*;

fn wait_until(done: impl Fn() -> bool) {
    let context = glib::MainContext::default();
    let deadline = std::time::Instant::now() + Duration::from_secs(4);
    while !done() {
        while context.pending() {
            context.iteration(false);
        }
        assert!(
            std::time::Instant::now() < deadline,
            "animation did not settle"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn replacement_cancels_stale_callbacks_but_independent_operations_all_complete() {
    crate::test_support::gtk_test(
        "ui::motion::tests::replacement_cancels_stale_callbacks_but_independent_operations_all_complete",
        || {
            gtk::Settings::default()
                .expect("settings")
                .set_gtk_enable_animations(true);
            for reduced in [false, true] {
                set_reduce_motion(reduced);
                let widget = gtk::Box::new(gtk::Orientation::Vertical, 0);
                let stale = Rc::new(Cell::new(0));
                let seen = stale.clone();
                animate(
                    &widget,
                    Some("replace"),
                    PANEL,
                    |_, _| {},
                    move || seen.set(seen.get() + 1),
                );
                let completed = Rc::new(Cell::new(0));
                let seen = completed.clone();
                animate(
                    &widget,
                    Some("replace"),
                    PANEL,
                    |_, _| {},
                    move || seen.set(seen.get() + 1),
                );
                for _ in 0..2 {
                    let seen = completed.clone();
                    animate(
                        &widget,
                        None,
                        PANEL,
                        |_, _| {},
                        move || seen.set(seen.get() + 1),
                    );
                }
                assert_eq!(completed.get(), 0, "completion must leave input dispatch");
                wait_until(|| completed.get() == 3);
                assert_eq!(stale.get(), 0);
            }
        },
    );
}

#[test]
fn mapped_animation_applies_feedback_immediately_and_settles_after_stalls_once() {
    crate::test_support::gtk_test(
        "ui::motion::tests::mapped_animation_applies_feedback_immediately_and_settles_after_stalls_once",
        || {
            set_reduce_motion(false);
            gtk::Settings::default()
                .expect("settings")
                .set_gtk_enable_animations(true);
            let widget = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let window = gtk::Window::builder().child(&widget).build();
            window.present();
            wait_until(|| widget.is_mapped());
            let frames = Rc::new(RefCell::new(Vec::new()));
            let seen = frames.clone();
            let completed = Rc::new(Cell::new(0));
            let done = completed.clone();
            animate(
                &widget,
                Some("paint"),
                Duration::from_millis(90),
                move |_, elapsed| seen.borrow_mut().push(elapsed),
                move || done.set(done.get() + 1),
            );
            assert_eq!(frames.borrow().as_slice(), &[Duration::ZERO]);
            std::thread::sleep(Duration::from_millis(150));
            wait_until(|| completed.get() > 0);
            assert_eq!(frames.borrow().first(), Some(&Duration::ZERO));
            assert!(frames.borrow().len() > 1);
            window.destroy();
            let settled = Rc::new(Cell::new(false));
            let done = settled.clone();
            glib::timeout_add_local_once(Duration::from_millis(150), move || done.set(true));
            wait_until(|| settled.get());
            assert_eq!(completed.get(), 1);
        },
    );
}

#[test]
fn reduced_motion_settles_live_and_rebuilt_views_without_settings() {
    crate::test_support::gtk_test(
        "ui::motion::tests::reduced_motion_settles_live_and_rebuilt_views_without_settings",
        || {
            let manager = crate::ui::preferences::PreferenceManager::shared();
            gtk::Settings::default()
                .expect("settings")
                .set_gtk_enable_animations(true);
            manager.set_reduce_motion(false);
            let first = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let second = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let first_window = gtk::Window::builder().child(&first).build();
            let second_window = gtk::Window::builder().child(&second).build();
            let completed = Rc::new(Cell::new(0));
            let done = completed.clone();
            animate(
                &first,
                Some("live"),
                PANEL,
                |_, _| {},
                move || done.set(done.get() + 1),
            );
            manager.set_reduce_motion(true);
            for widget in [&first, &second] {
                let done = completed.clone();
                animate(
                    widget,
                    Some("live"),
                    PANEL,
                    |_, _| {},
                    move || done.set(done.get() + 1),
                );
            }
            let rebuilt = gtk::Box::new(gtk::Orientation::Vertical, 0);
            second_window.set_child(Some(&rebuilt));
            let done = completed.clone();
            animate(
                &rebuilt,
                Some("live"),
                PANEL,
                |_, _| {},
                move || done.set(done.get() + 1),
            );
            wait_until(|| completed.get() == 3);
            assert_eq!(
                completed.get(),
                3,
                "the replaced old animation must not finish twice"
            );
            manager.set_reduce_motion(false);
            first_window.destroy();
            second_window.destroy();
        },
    );
}

#[test]
fn emphasized_deceleration_has_expected_endpoints() {
    assert!(emphasized_deceleration(0.0).abs() < 0.0001);
    assert!((emphasized_deceleration(1.0) - 1.0).abs() < 0.0001);
}

#[test]
fn emphasized_deceleration_is_monotonic_and_front_loaded() {
    let samples: Vec<_> = (0..=20)
        .map(|sample| emphasized_deceleration(f64::from(sample) / 20.0))
        .collect();

    assert!(samples.windows(2).all(|pair| pair[0] <= pair[1]));
    assert!(emphasized_deceleration(0.5) > 0.8);
}
