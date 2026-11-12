// SPDX-License-Identifier: MIT

use super::*;

fn render(left: &str, right: &str) -> String {
    diff(left, right, &AtomicBool::new(false)).expect("bounded comparison")
}

#[test]
fn recognizes_equal_text_and_normalizes_only_crlf() {
    for (left, right) in [
        ("", ""),
        ("a\n", "a\n"),
        ("a\r\nb\r\n", "a\nb\n"),
        ("日本語", "日本語"),
    ] {
        assert!(render(left, right).is_empty());
    }
    assert!(!render("a\r", "a").is_empty());
}

#[test]
fn reports_insertions_deletions_and_empty_file_ranges() {
    for (left, right, expected) in [
        ("", "new\n", "@@ -0,0 +1,1 @@\n+new\n"),
        ("old\n", "", "@@ -1,1 +0,0 @@\n-old\n"),
        ("a\nb\n", "a\nc\n", "@@ -1,2 +1,2 @@\n a\n-b\n+c\n"),
        ("a\n", "a\nb\n", "@@ -1,1 +1,2 @@\n a\n+b\n"),
        ("a\nb\n", "b\n", "@@ -1,2 +1,1 @@\n-a\n b\n"),
    ] {
        assert_eq!(render(left, right), expected);
    }
}

#[test]
fn retains_missing_final_newline_as_a_real_change() {
    assert_eq!(
        render("a", "a\n"),
        "@@ -1,1 +1,1 @@\n-a\n\\ No newline at end of file\n+a\n"
    );
    assert_eq!(
        render("a\n", "a"),
        "@@ -1,1 +1,1 @@\n-a\n+a\n\\ No newline at end of file\n"
    );
}

#[test]
fn uses_original_line_numbers_when_trimming_unchanged_context() {
    let prefix = "one\ntwo\nthree\nfour\nfive\n";
    let left = format!("{prefix}old\nsix\nseven\neight\nnine\n");
    let right = format!("{prefix}new\nsix\nseven\neight\nnine\n");
    assert_eq!(
        render(&left, &right),
        "@@ -3,7 +3,7 @@\n three\n four\n five\n-old\n+new\n six\n seven\n eight\n"
    );
}

#[test]
fn repeated_and_reordered_lines_reconstruct_both_inputs() {
    // Consume both sides of the hunk, independently of the algorithm's tie-breaking.
    let candidates = [
        "",
        "a\n",
        "a\na\n",
        "a\nb\na\n",
        "b\na\nb\n",
        "a\nb\n",
        "b\na\n",
        "雪\n🌲\n",
    ];
    for left in candidates {
        for right in candidates {
            let output = render(left, right);
            if left == right {
                assert!(output.is_empty());
                continue;
            }
            let mut old = String::new();
            let mut new = String::new();
            for line in output.split_inclusive('\n').skip(1) {
                match line.as_bytes()[0] {
                    b' ' => {
                        old.push_str(&line[1..]);
                        new.push_str(&line[1..]);
                    }
                    b'-' => old.push_str(&line[1..]),
                    b'+' => new.push_str(&line[1..]),
                    _ => panic!("unexpected hunk line"),
                }
            }
            assert_eq!(old, left);
            assert_eq!(new, right);
        }
    }
}

#[test]
fn rejects_large_inputs_line_counts_work_and_output_before_publication() {
    let cancel = AtomicBool::new(false);
    assert!(matches!(
        diff(&"x".repeat(MAX_BYTES as usize + 1), "", &cancel),
        Err(CompareError::TooLarge)
    ));
    assert!(matches!(
        diff(&"x\n".repeat(MAX_LINES + 1), "", &cancel),
        Err(CompareError::TooLarge)
    ));
    assert!(matches!(
        diff(&"a\n".repeat(1500), &"b\n".repeat(1500), &cancel),
        Err(CompareError::TooComplex)
    ));
    assert!(matches!(
        diff(&"x".repeat(MAX_OUTPUT), "y", &cancel),
        Err(CompareError::TooComplex)
    ));
}

#[test]
fn pre_cancelled_requests_do_not_read_or_publish_even_equal_inputs() {
    let cancel = AtomicBool::new(true);
    assert!(matches!(
        diff("same", "same", &cancel),
        Err(CompareError::Cancelled)
    ));
    assert!(matches!(
        read_text(Path::new("missing-file"), &cancel),
        Err(CompareError::Cancelled)
    ));
}

#[test]
fn reads_unicode_without_writing_sources_and_releases_handles() {
    let dir = tempfile::tempdir().expect("fixture");
    let path = dir.path().join("日本語.txt");
    let content = "雪\r\n🌲\n";
    std::fs::write(&path, content).expect("source");
    assert_eq!(
        read_text(&path, &AtomicBool::new(false)).expect("read"),
        content
    );
    assert_eq!(
        std::fs::read(&path).expect("unchanged source"),
        content.as_bytes()
    );
    std::fs::remove_file(path).expect("reader released the file");
}

#[test]
fn rejects_binary_non_utf8_directories_missing_and_oversized_files() {
    let dir = tempfile::tempdir().expect("fixture");
    let cancel = AtomicBool::new(false);
    for (name, bytes) in [
        ("nul.txt", vec![b'a', 0, b'b']),
        ("invalid.txt", vec![0xff, 0xfe]),
    ] {
        let path = dir.path().join(name);
        std::fs::write(&path, &bytes).expect("source");
        assert!(matches!(
            read_text(&path, &cancel),
            Err(CompareError::Unsupported)
        ));
        assert_eq!(std::fs::read(path).expect("unchanged"), bytes);
    }
    assert!(matches!(
        read_text(dir.path(), &cancel),
        Err(CompareError::Unsupported)
    ));
    assert!(matches!(
        read_text(&dir.path().join("missing"), &cancel),
        Err(CompareError::Io(_))
    ));
    let large = dir.path().join("large.txt");
    std::fs::File::create(&large)
        .expect("large source")
        .set_len(MAX_BYTES + 1)
        .expect("size");
    assert!(matches!(
        read_text(&large, &cancel),
        Err(CompareError::TooLarge)
    ));
}

#[test]
fn only_one_comparison_can_run_and_completion_releases_the_slot() {
    let permit = ComparisonPermit::reserve().expect("idle slot");
    assert!(ComparisonPermit::reserve().is_none());
    let dir = tempfile::tempdir().expect("fixture");
    let left = dir.path().join("left");
    let right = dir.path().join("right");
    std::fs::write(&left, "before\n").expect("left");
    std::fs::write(&right, "after\n").expect("right");
    assert!(
        compare(&permit, &left, &right, &AtomicBool::new(false))
            .expect("compare")
            .contains("+after\n")
    );
    drop(permit);
    assert!(ComparisonPermit::reserve().is_some());
}
