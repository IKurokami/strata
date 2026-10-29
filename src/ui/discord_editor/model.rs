// SPDX-License-Identifier: MIT

use std::ops::Range;

pub(super) const BYTE_LIMIT: usize = 64 * 1024;
pub(super) const RUN_LIMIT: usize = 4096;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Mode {
    Ansi,
    #[default]
    Markdown,
    Code,
    Plain,
}

impl Mode {
    pub const ALL: [Self; 4] = [Self::Ansi, Self::Markdown, Self::Code, Self::Plain];

    pub fn label(self) -> &'static str {
        match self {
            Self::Ansi => "ANSI",
            Self::Markdown => "Markdown",
            Self::Code => "Code",
            Self::Plain => "Plain text",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct Color(pub u8, pub u8, pub u8);

impl Color {
    pub fn hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }
}

// These are document colors, independent of Strata's interface theme.
pub(super) const PALETTE: [Color; 8] = [
    Color(79, 84, 92),
    Color(220, 50, 47),
    Color(133, 153, 0),
    Color(181, 137, 0),
    Color(38, 139, 210),
    Color(211, 54, 130),
    Color(42, 161, 152),
    Color(255, 255, 255),
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(super) struct Style {
    pub bold: bool,
    pub underline: bool,
    pub italic: bool,
    pub strike: bool,
    pub foreground: Option<Color>,
    pub background: Option<Color>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Run {
    pub text: String,
    pub style: Style,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Document {
    pub runs: Vec<Run>,
}

impl Document {
    pub fn plain(text: &str) -> Self {
        Self {
            runs: vec![Run {
                text: text.to_owned(),
                style: Style::default(),
            }],
        }
    }

    pub fn text(&self) -> String {
        self.runs.iter().map(|run| run.text.as_str()).collect()
    }

    pub fn push(&mut self, text: &str, style: Style) {
        if text.is_empty() {
            return;
        }
        if let Some(last) = self.runs.last_mut().filter(|last| last.style == style) {
            last.text.push_str(text);
        } else {
            self.runs.push(Run {
                text: text.to_owned(),
                style,
            });
        }
    }

    pub fn edit(
        &self,
        range: Range<usize>,
        mut edit: impl FnMut(&mut Style, usize, usize),
    ) -> Self {
        let mut result = Self::default();
        let mut offset = 0;
        let count = range.end.saturating_sub(range.start);
        for run in &self.runs {
            for ch in run.text.chars() {
                let mut style = run.style;
                if range.contains(&offset) {
                    edit(&mut style, offset - range.start, count);
                }
                result.push(ch.encode_utf8(&mut [0; 4]), style);
                offset += 1;
            }
        }
        result
    }
}

fn body(source: &str) -> Option<(&str, &str)> {
    let source = source.trim_end_matches(['\r', '\n']);
    let (header, rest) = source.split_once('\n')?;
    let language = header.trim_end_matches('\r').strip_prefix("```")?;
    let text = rest
        .strip_suffix("\n```")
        .or_else(|| rest.strip_suffix("\r\n```"))?;
    Some((language, text.strip_suffix('\r').unwrap_or(text)))
}

pub(super) fn initial(source: &str) -> Result<(Mode, Document, String), &'static str> {
    if let Some((language, text)) = body(source) {
        if language.eq_ignore_ascii_case("ansi") {
            return Ok((Mode::Ansi, parse_ansi(text)?, String::new()));
        }
        return Ok((Mode::Code, Document::plain(text), language.to_owned()));
    }
    Ok((Mode::Markdown, Document::plain(source), String::new()))
}

pub(super) fn parse_ansi(source: &str) -> Result<Document, &'static str> {
    let mut result = Document::default();
    let mut style = Style::default();
    let mut remaining = source;
    while let Some(position) = remaining.find('\x1b') {
        result.push(&remaining[..position], style);
        let tail = remaining[position..]
            .strip_prefix("\x1b[")
            .ok_or("Unsupported ANSI sequence")?;
        let end = tail
            .find('m')
            .filter(|end| *end <= 80)
            .ok_or("Unsupported ANSI sequence")?;
        let codes: Vec<u16> = tail[..end]
            .split(';')
            .map(|part| if part.is_empty() { Ok(0) } else { part.parse() })
            .collect::<Result<_, _>>()
            .map_err(|_| "Unsupported ANSI sequence")?;
        let mut index = 0;
        while index < codes.len() {
            let code = codes[index];
            match code {
                0 => style = Style::default(),
                1 => style.bold = true,
                22 => style.bold = false,
                3 => style.italic = true,
                4 => style.underline = true,
                9 => style.strike = true,
                23 => style.italic = false,
                24 => style.underline = false,
                29 => style.strike = false,
                30..=37 => style.foreground = Some(PALETTE[(code - 30) as usize]),
                40..=47 => style.background = Some(PALETTE[(code - 40) as usize]),
                90..=97 => style.foreground = Some(indexed_color((code - 90 + 8) as u8)),
                100..=107 => style.background = Some(indexed_color((code - 100 + 8) as u8)),
                39 => style.foreground = None,
                49 => style.background = None,
                38 | 48 => {
                    let color = match codes.get(index + 1) {
                        Some(2) if codes.len() > index + 4 => {
                            let values = &codes[index + 2..index + 5];
                            if values.iter().any(|value| *value > 255) {
                                return Err("Unsupported ANSI sequence");
                            }
                            index += 4;
                            Color(values[0] as u8, values[1] as u8, values[2] as u8)
                        }
                        Some(5) if codes.get(index + 2).is_some_and(|value| *value <= 255) => {
                            index += 2;
                            indexed_color(codes[index] as u8)
                        }
                        _ => return Err("Unsupported ANSI sequence"),
                    };
                    if code == 38 {
                        style.foreground = Some(color);
                    } else {
                        style.background = Some(color);
                    }
                }
                _ => return Err("Unsupported ANSI sequence"),
            }
            index += 1;
        }
        if result.runs.len() > RUN_LIMIT {
            return Err("Text is too large for Discord editing");
        }
        remaining = &tail[end + 1..];
    }
    result.push(remaining, style);
    if result.runs.len() > RUN_LIMIT || result.text().len() > BYTE_LIMIT {
        return Err("Text is too large for Discord editing");
    }
    Ok(result)
}

fn indexed_color(index: u8) -> Color {
    match index {
        0..=7 => PALETTE[index as usize],
        8..=15 => [
            Color(128, 128, 128),
            Color(255, 85, 85),
            Color(85, 255, 85),
            Color(255, 255, 85),
            Color(85, 85, 255),
            Color(255, 85, 255),
            Color(85, 255, 255),
            Color(255, 255, 255),
        ][(index - 8) as usize],
        16..=231 => {
            let index = index - 16;
            let component = |n: u8| if n == 0 { 0 } else { 55 + 40 * n };
            Color(
                component(index / 36),
                component(index / 6 % 6),
                component(index % 6),
            )
        }
        _ => {
            let gray = 8 + 10 * (index - 232);
            Color(gray, gray, gray)
        }
    }
}

pub(super) fn export(
    mode: Mode,
    document: &Document,
    language: &str,
) -> Result<String, &'static str> {
    let text = document.text();
    if text.len() > BYTE_LIMIT || document.runs.len() > RUN_LIMIT {
        return Err("Text is too large for Discord editing");
    }
    if !matches!(mode, Mode::Ansi | Mode::Code) {
        return Ok(text);
    }
    if text.contains("```") {
        return Err("Text contains a code-block delimiter");
    }
    let mut body = String::new();
    let language = if mode == Mode::Ansi {
        for run in &document.runs {
            body.push_str("\x1b[0");
            let style = run.style;
            for (enabled, code) in [
                (style.bold, 1),
                (style.italic, 3),
                (style.underline, 4),
                (style.strike, 9),
            ] {
                if enabled {
                    body.push_str(&format!(";{code}"));
                }
            }
            for (color, code) in [(style.background, 48), (style.foreground, 38)] {
                if let Some(Color(r, g, b)) = color {
                    if let Some(index) = PALETTE.iter().position(|value| Some(*value) == color) {
                        body.push_str(&format!(
                            ";{}",
                            if code == 48 { 40 + index } else { 30 + index }
                        ));
                    } else {
                        body.push_str(&format!(";{code};2;{r};{g};{b}"));
                    }
                }
            }
            body.push('m');
            body.push_str(&run.text);
        }
        body.push_str("\x1b[0m");
        "ansi"
    } else {
        if language.len() > 32
            || !language
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '-' | '_' | '#'))
        {
            return Err("Invalid code language");
        }
        body = text;
        language
    };
    let output = format!("```{language}\n{body}\n```");
    if output.len() > BYTE_LIMIT {
        return Err("Text is too large for Discord editing");
    }
    Ok(output)
}

#[derive(Clone, Copy)]
pub(super) enum MarkdownAction {
    Wrap(&'static str, &'static str),
    Lines(&'static str),
    Numbered,
    BlockQuote,
}

pub(super) const MARKDOWN_TOOLS: &[(&str, MarkdownAction)] = &[
    ("Bold", MarkdownAction::Wrap("**", "**")),
    ("Italic", MarkdownAction::Wrap("*", "*")),
    ("Underline", MarkdownAction::Wrap("__", "__")),
    ("Strikethrough", MarkdownAction::Wrap("~~", "~~")),
    ("Spoiler", MarkdownAction::Wrap("||", "||")),
    ("Inline code", MarkdownAction::Wrap("`", "`")),
    ("Link", MarkdownAction::Wrap("[", "](https://)")),
    ("Heading 1", MarkdownAction::Lines("# ")),
    ("Heading 2", MarkdownAction::Lines("## ")),
    ("Heading 3", MarkdownAction::Lines("### ")),
    ("Subtext", MarkdownAction::Lines("-# ")),
    ("Quote", MarkdownAction::Lines("> ")),
    ("Block quote", MarkdownAction::BlockQuote),
    ("Bullet list", MarkdownAction::Lines("- ")),
    ("Numbered list", MarkdownAction::Numbered),
    ("Code block", MarkdownAction::Wrap("```\n", "\n```")),
];

pub(super) fn markdown_edit(
    text: &str,
    range: Range<usize>,
    action: MarkdownAction,
) -> (String, Range<usize>) {
    let boundaries: Vec<usize> = text
        .char_indices()
        .map(|(index, _)| index)
        .chain(std::iter::once(text.len()))
        .collect();
    let mut start = range.start.min(boundaries.len() - 1);
    let mut end = range.end.max(start).min(boundaries.len() - 1);
    if matches!(
        action,
        MarkdownAction::Lines(_) | MarkdownAction::Numbered | MarkdownAction::BlockQuote
    ) {
        start = text[..boundaries[start]]
            .rfind('\n')
            .map_or(0, |position| text[..position].chars().count() + 1);
        end += text[boundaries[end]..]
            .chars()
            .position(|ch| ch == '\n')
            .unwrap_or(text[boundaries[end]..].chars().count());
    }
    let selected = &text[boundaries[start]..boundaries[end]];
    let (replacement, offset) = match action {
        MarkdownAction::Wrap(before, after) => {
            (format!("{before}{selected}{after}"), before.chars().count())
        }
        MarkdownAction::Lines(prefix) => (
            selected
                .split('\n')
                .map(|line| format!("{prefix}{line}"))
                .collect::<Vec<_>>()
                .join("\n"),
            0,
        ),
        MarkdownAction::Numbered => (
            selected
                .split('\n')
                .enumerate()
                .map(|(index, line)| format!("{}. {line}", index + 1))
                .collect::<Vec<_>>()
                .join("\n"),
            0,
        ),
        MarkdownAction::BlockQuote => (format!(">>> {selected}"), 4),
    };
    let selection = start + offset
        ..start + replacement.chars().count()
            - match action {
                MarkdownAction::Wrap(_, after) => after.chars().count(),
                _ => 0,
            };
    (
        format!(
            "{}{}{}",
            &text[..boundaries[start]],
            replacement,
            &text[boundaries[end]..]
        ),
        selection,
    )
}

#[cfg(test)]
mod tests;
