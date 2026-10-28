// SPDX-License-Identifier: MIT

use crate::services::smart_search::result::{ContentLocator, ExtractedSegment, Extraction};
use calamine::Reader;
use gtk::gio::{self, prelude::*};
use std::{
    fs::File,
    io::{Cursor, Read},
    path::Path,
};

const TEXT_LIMIT: u64 = 4 * 1024 * 1024;
const CHUNK: usize = 1600;

pub(super) fn read(path: &Path, cursor: u32) -> Result<Vec<u8>, String> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let extraction = match extension.as_str() {
        "pdf" => pdf(path, cursor)?,
        "xlsx" | "xls" | "ods" => workbook(path, cursor)?,
        "docx" | "odt" | "pptx" | "odp" => office(path, cursor, &extension)?,
        _ => {
            let mut bytes = Vec::new();
            File::open(path)
                .map_err(|error| error.to_string())?
                .take(TEXT_LIMIT + 1)
                .read_to_end(&mut bytes)
                .map_err(|error| error.to_string())?;
            if bytes.len() as u64 > TEXT_LIMIT {
                return Err("Text exceeds the content indexing budget".into());
            }
            let text = if bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xff]) {
                let little = bytes[0] == 0xff;
                if bytes.len() % 2 != 0 {
                    return Err("Invalid UTF-16 text".into());
                }
                String::from_utf16(
                    &bytes[2..]
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|pair| {
                            if little {
                                u16::from_le_bytes([pair[0], pair[1]])
                            } else {
                                u16::from_be_bytes([pair[0], pair[1]])
                            }
                        })
                        .collect::<Vec<_>>(),
                )
                .map_err(|_| "Invalid UTF-16 text")?
            } else {
                std::str::from_utf8(&bytes)
                    .map_err(|_| "Content is not UTF-8 text")?
                    .trim_start_matches('\u{feff}')
                    .to_owned()
            };
            if text.contains('\0') {
                return Err("Binary content is not indexed as text".into());
            }
            text_chunks(
                &text,
                cursor,
                crate::services::smart_search::policy::code(path),
                None,
            )
        }
    };
    serde_json::to_vec(&extraction).map_err(|error| error.to_string())
}

pub(crate) fn text_chunks(text: &str, cursor: u32, code: bool, page: Option<u32>) -> Extraction {
    let chars: Vec<_> = text.chars().take(TEXT_LIMIT as usize).collect();
    let first = cursor as usize;
    let end = chars.len().min(first.saturating_add(CHUNK * 16));
    let mut segments = Vec::new();
    let mut offset = first;
    while offset < end {
        let last = (offset + CHUNK).min(end);
        let piece: String = chars[offset..last].iter().collect();
        let anchor: String = piece.chars().take(120).collect();
        if !piece.trim().is_empty() {
            let locator = if let Some(page_0) = page {
                ContentLocator::Pdf {
                    page_0,
                    start_char: offset as u32,
                    end_char: last as u32,
                    anchor,
                }
            } else if code {
                let line_1 = 1 + chars[..offset]
                    .iter()
                    .filter(|character| **character == '\n')
                    .count() as u32;
                ContentLocator::Code {
                    line_1,
                    end_line_1: line_1
                        + piece.chars().filter(|character| *character == '\n').count() as u32,
                    anchor,
                }
            } else {
                ContentLocator::Text {
                    chunk: (offset / CHUNK) as u32,
                    start_char: offset as u32,
                    end_char: last as u32,
                    anchor,
                }
            };
            segments.push(ExtractedSegment {
                locator,
                text: piece,
                modality: if code { "code" } else { "text" }.into(),
            });
        }
        offset = last;
    }
    Extraction {
        segments,
        next: (end < chars.len()).then_some(end as u32),
    }
}

const CURSOR_BITS: u32 = 21;
const CURSOR_MASK: u32 = (1 << CURSOR_BITS) - 1;
fn cursor_parts(cursor: u32) -> (usize, u32) {
    ((cursor >> CURSOR_BITS) as usize, cursor & CURSOR_MASK)
}
fn next_part(part: usize, offset: u32) -> Result<u32, String> {
    if part >= 1024 || offset > CURSOR_MASK {
        return Err("Document exceeds the resumable indexing budget".into());
    }
    Ok(((part as u32) << CURSOR_BITS) | offset)
}

fn pdf(path: &Path, cursor: u32) -> Result<Extraction, String> {
    let uri = gio::File::for_path(path).uri();
    let document = poppler::Document::from_file(&uri, None).map_err(|error| error.to_string())?;
    let pages = document.n_pages().max(0) as usize;
    if pages > 1024 {
        return Err("PDF exceeds the page indexing budget".into());
    }
    let (index, offset) = cursor_parts(cursor);
    let page = document
        .page(index as i32)
        .ok_or("PDF page is unavailable")?;
    let text = page.text().map(|text| text.to_string()).unwrap_or_default();
    let mut extracted = text_chunks(&text, offset, false, Some(index as u32));
    if offset == 0 && text.trim().is_empty() {
        extracted.segments.push(ExtractedSegment {
            locator: ContentLocator::Pdf {
                page_0: index as u32,
                start_char: 0,
                end_char: 0,
                anchor: String::new(),
            },
            text: String::new(),
            modality: "image".into(),
        });
    }
    extracted.next = if let Some(next) = extracted.next {
        Some(next_part(index, next)?)
    } else if index + 1 < pages {
        Some(next_part(index + 1, 0)?)
    } else {
        None
    };
    Ok(extracted)
}

fn workbook(path: &Path, cursor: u32) -> Result<Extraction, String> {
    let mut workbook = calamine::open_workbook_auto(path).map_err(|error| error.to_string())?;
    let names = workbook.sheet_names().to_vec();
    if names.len() > 1024 {
        return Err("Workbook exceeds the sheet indexing budget".into());
    }
    let (sheet, first) = cursor_parts(cursor);
    let name = names.get(sheet).ok_or("Worksheet is unavailable")?;
    let range = workbook
        .worksheet_range(name)
        .map_err(|error| error.to_string())?;
    let start = range.start().unwrap_or((0, 0));
    let end = range.height().min(first as usize + 64);
    let mut segments = Vec::new();
    for (row_index, row) in range.rows().enumerate().take(end).skip(first as usize) {
        // Each cell retains its own exact sheet/row/column locator.
        for (column, cell) in row.iter().enumerate() {
            let text = cell.to_string();
            if text.trim().is_empty() {
                continue;
            }
            if segments.len() >= 1024 || text.len() > 16000 {
                return Err("Worksheet batch exceeds the content indexing budget".into());
            }
            segments.push(ExtractedSegment {
                locator: ContentLocator::Sheet {
                    name: name.clone(),
                    row_1: start.0 + row_index as u32 + 1,
                    col_1: start.1 + column as u32 + 1,
                    anchor: text.chars().take(120).collect(),
                },
                text,
                modality: "text".into(),
            });
        }
    }
    let next = if end < range.height() {
        Some(next_part(sheet, end as u32)?)
    } else if sheet + 1 < names.len() {
        Some(next_part(sheet + 1, 0)?)
    } else {
        None
    };
    Ok(Extraction { segments, next })
}

fn office(path: &Path, cursor: u32, extension: &str) -> Result<Extraction, String> {
    let source = File::open(path).map_err(|error| error.to_string())?;
    let mut archive = zip::ZipArchive::new(source).map_err(|error| error.to_string())?;
    if archive.len() > 10000 {
        return Err("Office archive exceeds the entry budget".into());
    }
    let mut names: Vec<_> = archive
        .file_names()
        .filter(|name| {
            if extension == "pptx" {
                name.starts_with("ppt/slides/slide")
                    && name.ends_with(".xml")
                    && !name.contains("_rels")
            } else {
                matches!(*name, "word/document.xml" | "content.xml")
            }
        })
        .map(str::to_owned)
        .collect();
    names.sort_by_key(|name| {
        name.chars()
            .filter(char::is_ascii_digit)
            .collect::<String>()
            .parse::<u32>()
            .unwrap_or(0)
    });
    let (part, offset) = cursor_parts(cursor);
    let name = names
        .get(if extension == "odp" { 0 } else { part })
        .ok_or("Office document part is unavailable")?;
    let mut entry = archive.by_name(name).map_err(|error| error.to_string())?;
    if entry.size() > TEXT_LIMIT {
        return Err("Office XML exceeds the text budget".into());
    }
    let mut bytes = Vec::new();
    entry
        .by_ref()
        .take(TEXT_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    let pages = if extension == "odp" {
        odp_pages(&bytes)?
    } else {
        vec![xml_text(&bytes)?]
    };
    let text = pages
        .get(if extension == "odp" { part } else { 0 })
        .ok_or("Office page is unavailable")?;
    let mut extracted = text_chunks(text, offset, false, None);
    if matches!(extension, "pptx" | "odp") {
        for segment in &mut extracted.segments {
            segment.locator = ContentLocator::Slide {
                slide_0: part as u32,
                anchor: segment.text.chars().take(120).collect(),
            };
        }
    }
    let count = if extension == "odp" {
        pages.len()
    } else {
        names.len()
    };
    extracted.next = if let Some(next) = extracted.next {
        Some(next_part(part, next)?)
    } else if part + 1 < count {
        Some(next_part(part + 1, 0)?)
    } else {
        None
    };
    Ok(extracted)
}

fn odp_pages(bytes: &[u8]) -> Result<Vec<String>, String> {
    use quick_xml::events::Event;
    let mut reader = quick_xml::Reader::from_reader(Cursor::new(bytes));
    let mut buffer = Vec::new();
    let mut pages = Vec::new();
    let mut current = None::<String>;
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(value)) if value.name().as_ref() == b"draw:page" => {
                current = Some(String::new())
            }
            Ok(Event::End(value)) if value.name().as_ref() == b"draw:page" => {
                if let Some(text) = current.take() {
                    pages.push(text);
                }
            }
            Ok(Event::Text(value)) => {
                if let Some(text) = current.as_mut() {
                    let decoded = value.decode().map_err(|error| error.to_string())?;
                    text.push_str(
                        &quick_xml::escape::unescape(&decoded)
                            .map_err(|error| error.to_string())?,
                    );
                }
            }
            Ok(Event::GeneralRef(value)) => {
                if let Some(text) = current.as_mut() {
                    text.push_str(&xml_reference(value.as_ref())?);
                }
            }
            Ok(Event::CData(value)) => {
                if let Some(text) = current.as_mut() {
                    text.push_str(&value.decode().map_err(|error| error.to_string())?);
                }
            }
            Ok(Event::End(value)) if value.local_name().as_ref() == b"p" => {
                if let Some(text) = current.as_mut() {
                    text.push('\n');
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(error) => return Err(error.to_string()),
        }
        buffer.clear();
        if pages.len() > 1024 {
            return Err("Presentation exceeds the page indexing budget".into());
        }
    }
    Ok(pages)
}

fn xml_reference(name: &[u8]) -> Result<String, String> {
    let name = std::str::from_utf8(name).map_err(|error| error.to_string())?;
    quick_xml::escape::unescape(&format!("&{name};"))
        .map(|value| value.into_owned())
        .map_err(|error| error.to_string())
}

fn xml_text(bytes: &[u8]) -> Result<String, String> {
    use quick_xml::events::Event;
    let mut reader = quick_xml::Reader::from_reader(Cursor::new(bytes));
    let mut text = String::new();
    let mut buffer = Vec::new();
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Text(value)) => {
                let value = value.decode().map_err(|error| error.to_string())?;
                text.push_str(
                    &quick_xml::escape::unescape(&value).map_err(|error| error.to_string())?,
                );
            }
            Ok(Event::GeneralRef(value)) => text.push_str(&xml_reference(value.as_ref())?),
            Ok(Event::CData(value)) => {
                text.push_str(&value.decode().map_err(|error| error.to_string())?)
            }
            Ok(Event::End(value))
                if matches!(
                    value.local_name().as_ref(),
                    b"p" | b"row" | b"table-row" | b"tr"
                ) =>
            {
                text.push('\n')
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(error) => return Err(error.to_string()),
        }
        buffer.clear();
        if text.len() > TEXT_LIMIT as usize {
            return Err("Office text exceeds the indexing budget".into());
        }
    }
    Ok(text)
}

#[cfg(test)]
mod tests;
