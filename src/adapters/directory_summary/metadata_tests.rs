// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn missing_sizes_keep_known_bytes_and_report_partial_measurement() {
    crate::test_support::gtk_test_with_env(
        "adapters::directory_summary::metadata_tests::missing_sizes_keep_known_bytes_and_report_partial_measurement",
        [("G_DEBUG", "fatal-criticals")],
        || {
            let budget = Rc::new(MeasurementBudget {
                deadline: Instant::now() + TIME_BUDGET,
                max_depth: MAX_DEPTH,
                total: Cell::default(),
                reported: Cell::default(),
                on_progress: Box::new(|_| {}),
            });
            let context = glib::MainContext::new();
            for (size, expected_bytes, partial) in
                [(None, 0, true), (Some(0), 0, false), (Some(7), 7, false)]
            {
                let info = gio::FileInfo::new();
                info.set_file_type(gio::FileType::Regular);
                info.set_is_hidden(false);
                info.set_is_symlink(false);
                if let Some(size) = size {
                    info.set_size(size);
                }
                let summary = context
                    .block_on(measure_entry(
                        gio::File::for_path(std::env::temp_dir().join("strata-size-fixture")),
                        info,
                        0,
                        false,
                        budget.clone(),
                    ))
                    .expect("regular file measurement");
                assert_eq!(summary.item_count, 1);
                assert_eq!(summary.total_size, expected_bytes);
                assert_eq!(summary.issues.unreadable, partial);
                assert_eq!(summary.truncated(), partial);
            }
            assert_eq!(budget.total.get().item_count, 3);
            assert_eq!(budget.total.get().total_size, 7);
            assert!(budget.total.get().issues.unreadable);
        },
    );
}
