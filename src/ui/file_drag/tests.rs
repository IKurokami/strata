// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn diagnostics_preserve_dynamic_and_preset_drag_providers() {
    crate::test_support::gtk_test(
        "ui::file_drag::tests::diagnostics_preserve_dynamic_and_preset_drag_providers",
        || {
            for preset in [false, true] {
                let source = gtk::DragSource::new();
                let provider = gtk::gdk::ContentProvider::for_value(&"drag payload".to_value());
                let observe = install(&source, "test");
                if preset {
                    source.set_content(Some(&provider));
                } else {
                    let content = provider.clone();
                    source.connect_prepare(move |source, _, _| {
                        observe(source);
                        Some(content.clone())
                    });
                }
                let actual = source.emit_by_name::<Option<gtk::gdk::ContentProvider>>(
                    "prepare",
                    &[&0.0_f64, &0.0_f64],
                );
                assert_eq!(
                    actual,
                    Some(provider),
                    "diagnostics replaced the drag content"
                );
            }
        },
    );
}
