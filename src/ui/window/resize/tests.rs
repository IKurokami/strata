// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn border_hover_releases_the_content_cursor_on_exit_and_unmap() {
    crate::test_support::gtk_test(
        "ui::window::resize::tests::border_hover_releases_the_content_cursor_on_exit_and_unmap",
        || {
            let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let window = gtk::ApplicationWindow::builder()
                .decorated(false)
                .default_width(240)
                .default_height(160)
                .child(&content)
                .build();
            install(&window);
            window.present();
            let painted = Rc::new(std::cell::Cell::new(false));
            let seen = painted.clone();
            let clock = window.frame_clock().expect("window frame clock");
            let handler = clock.connect_after_paint(move |_| seen.set(true));
            window.queue_draw();
            let context = glib::MainContext::default();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while !painted.get() {
                assert!(
                    std::time::Instant::now() < deadline,
                    "resize fixture did not paint"
                );
                context.iteration(false);
            }
            clock.disconnect(handler);
            let controllers = window.observe_controllers();
            let motion = (0..controllers.n_items())
                .find_map(|index| {
                    controllers
                        .item(index)
                        .and_downcast::<gtk::EventControllerMotion>()
                })
                .expect("resize motion controller");
            let target = window
                .pick(1.0, 80.0, gtk::PickFlags::DEFAULT)
                .unwrap_or_else(|| window.clone().upcast());
            target.set_cursor_from_name(Some("crosshair"));
            let cursor = |widget: &gtk::Widget| {
                widget
                    .cursor()
                    .and_then(|cursor| cursor.name())
                    .map(|name| name.to_string())
            };
            motion.emit_by_name::<()>("enter", &[&1f64, &80f64]);
            assert_eq!(cursor(&target).as_deref(), Some("w-resize"));
            motion.emit_by_name::<()>("motion", &[&80f64, &80f64]);
            assert_eq!(cursor(&target).as_deref(), Some("crosshair"));
            let corner = window
                .pick(1.0, 1.0, gtk::PickFlags::DEFAULT)
                .unwrap_or_else(|| window.clone().upcast());
            corner.set_cursor_from_name(Some("crosshair"));
            motion.emit_by_name::<()>("motion", &[&1f64, &1f64]);
            assert_eq!(cursor(&corner).as_deref(), Some("nw-resize"));
            motion.emit_by_name::<()>("leave", &[]);
            assert_eq!(cursor(&corner).as_deref(), Some("crosshair"));
            motion.emit_by_name::<()>("motion", &[&1f64, &80f64]);
            window.set_visible(false);
            assert_eq!(cursor(&target).as_deref(), Some("crosshair"));
            window.destroy();
        },
    );
}
