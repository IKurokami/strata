// SPDX-License-Identifier: MIT

use super::model::{Document, Style};

pub(super) fn render(source: &str) -> Document {
    let mut document = Document::default();
    let mut code = false;
    let mut multi_quote = false;
    let mut budget = 1_000_000;
    for line in source.split_inclusive('\n') {
        if line.starts_with("```") {
            code = !code;
            continue;
        }
        if code {
            document.push(line, Style::default());
            continue;
        }
        let line = if let Some(rest) = line.strip_prefix(">>> ") {
            multi_quote = true;
            rest
        } else {
            line
        };
        let line = if multi_quote {
            line
        } else {
            line.strip_prefix("> ").unwrap_or(line)
        };
        let mut style = Style::default();
        let line = if let Some(rest) = line
            .strip_prefix("### ")
            .or_else(|| line.strip_prefix("## "))
            .or_else(|| line.strip_prefix("# "))
        {
            style.bold = true;
            rest
        } else {
            line.strip_prefix("-# ").unwrap_or(line)
        };
        inline(line, style, 0, &mut budget, &mut document);
    }
    document
}

fn inline(mut text: &str, style: Style, depth: u8, budget: &mut usize, document: &mut Document) {
    if depth > 8 {
        document.push(text, style);
        return;
    }
    while !text.is_empty() {
        if *budget == 0 {
            document.push(text, style);
            return;
        }
        let mut formatted = false;
        for marker in ["`", "***", "**", "__", "~~", "||", "*", "_"] {
            if let Some(tail) = text.strip_prefix(marker) {
                *budget = budget.saturating_sub(tail.len().max(1));
                if let Some(end) = tail.find(marker) {
                    let mut next = style;
                    match marker {
                        "***" => {
                            next.bold = true;
                            next.italic = true;
                        }
                        "**" => next.bold = true,
                        "__" => next.underline = true,
                        "~~" => next.strike = true,
                        "*" | "_" => next.italic = true,
                        _ => {}
                    }
                    if marker == "||" {
                        document.push(&"▇".repeat(tail[..end].chars().count()), style);
                    } else if marker == "`" {
                        document.push(&tail[..end], style);
                    } else {
                        inline(&tail[..end], next, depth + 1, budget, document);
                    }
                    text = &tail[end + marker.len()..];
                    formatted = true;
                    break;
                }
            }
        }
        if formatted {
            continue;
        }
        if let Some(tail) = text.strip_prefix('\\')
            && let Some(ch) = tail.chars().next()
        {
            document.push(ch.encode_utf8(&mut [0; 4]), style);
            text = &tail[ch.len_utf8()..];
            continue;
        }
        if let Some(tail) = text.strip_prefix('[') {
            *budget = budget.saturating_sub(tail.len().max(1));
            if let Some((label, rest)) = tail.split_once("](")
                && let Some(end) = rest.find(')')
            {
                *budget = budget.saturating_sub(rest.len());
                inline(
                    label,
                    Style {
                        underline: true,
                        ..style
                    },
                    depth + 1,
                    budget,
                    document,
                );
                text = &rest[end + 1..];
                continue;
            }
        }
        let Some(ch) = text.chars().next() else {
            break;
        };
        document.push(ch.encode_utf8(&mut [0; 4]), style);
        text = &text[ch.len_utf8()..];
    }
}
