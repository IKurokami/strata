// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn ansi_import_export_preserves_unicode_resets_rgb_and_backgrounds() {
    for source in [
        "```ansi\n\x1b[1;36m✦ NỮ HOÀNG\x1b[0m\n\x1b[38;2;245;251;255mT\x1b[38;2;84;183;243mi\x1b[0m\n```\n",
        "```ansi\r\n\x1b[4;41;32mDANICA\x1b[24;39;49m thường\r\n```\r\n",
        "```ansi\n\x1b[38;5;200mindexed\x1b[22;0m reset\n```",
    ] {
        let (mode, document, language) = initial(source).expect("valid ANSI");
        assert_eq!(mode, Mode::Ansi);
        let serialized = export(mode, &document, &language).expect("serialized ANSI");
        let (_, decoded, _) = initial(&serialized).expect("readable export");
        assert_eq!(decoded, document);
        assert_eq!(serialized.matches("```ansi").count(), 1);
        assert!(!document.text().contains('\x1b'));
    }
}

#[test]
fn ordinary_text_stays_unwrapped_until_an_export_mode_is_selected() {
    let source = "ảnh **mới**\nhello";
    let (mode, document, _) = initial(source).expect("ordinary text");
    assert_eq!(mode, Mode::Markdown);
    assert_eq!(export(mode, &document, "").expect("Markdown"), source);
    assert_eq!(export(Mode::Plain, &document, "").expect("plain"), source);
    assert_eq!(
        export(Mode::Code, &document, "rust").expect("code"),
        format!("```rust\n{source}\n```")
    );
    assert!(
        export(Mode::Ansi, &document, "")
            .expect("ANSI")
            .starts_with("```ansi\n")
    );
    assert!(export(Mode::Code, &document, "rust\n@everyone").is_err());
    assert!(export(Mode::Ansi, &Document::plain("text ``` tail"), "").is_err());
}

#[test]
fn ansi_selection_changes_preserve_unselected_and_mixed_formatting() {
    let document = parse_ansi("\x1b[1;31mảnh\x1b[0;4;32m Việt").expect("styled input");
    let changed = document.edit(1..5, |style, _, _| {
        style.background = Some(Color(10, 20, 30))
    });
    assert_eq!(changed.text(), document.text());
    let mut offset = 0;
    for run in &changed.runs {
        for _ in run.text.chars() {
            assert_eq!(
                run.style.background,
                if (1..5).contains(&offset) {
                    Some(Color(10, 20, 30))
                } else {
                    None
                }
            );
            assert_eq!(run.style.bold, offset < 3);
            assert_eq!(run.style.underline, offset >= 3);
            offset += 1;
        }
    }
    assert_eq!(changed.text().chars().count(), 8);
}

#[test]
fn malformed_or_excessive_ansi_is_rejected_without_interpreting_terminal_commands() {
    for text in [
        "\x1b]0;title\x07",
        "\x1b[2J",
        "\x1b[38;2;999;2;3m",
        "\x1b[38;2;1m",
        "\x1b[999m",
    ] {
        assert!(parse_ansi(text).is_err(), "must reject {text:?}");
    }
    assert!(parse_ansi(&"a".repeat(BYTE_LIMIT + 1)).is_err());
    assert!(parse_ansi(&"\x1b[31ma\x1b[32mb".repeat(RUN_LIMIT)).is_err());
}

#[test]
fn markdown_tools_handle_unicode_selections_lines_and_empty_cursors() {
    let (text, selection) = markdown_edit("ảnh Việt", 0..3, MarkdownAction::Wrap("**", "**"));
    assert_eq!(text, "**ảnh** Việt");
    assert_eq!(selection, 2..5);
    let (text, _) = markdown_edit("đầu\nhai\nba", 5..8, MarkdownAction::Lines("> "));
    assert_eq!(text, "đầu\n> hai\n> ba");
    let (text, _) = markdown_edit("một\nhai", 0..7, MarkdownAction::Numbered);
    assert_eq!(text, "1. một\n2. hai");
    let (text, selection) = markdown_edit("", 0..0, MarkdownAction::Wrap("||", "||"));
    assert_eq!(text, "||||");
    assert_eq!(selection, 2..2);
    let (text, _) = markdown_edit("trước\nđoạn sau", 8..9, MarkdownAction::BlockQuote);
    assert_eq!(text, "trước\n>>> đoạn sau");
}
