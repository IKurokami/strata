// SPDX-License-Identifier: MIT

use windows::{
    Globalization::Language,
    Graphics::Imaging::{BitmapAlphaMode, BitmapPixelFormat, SoftwareBitmap},
    Media::Ocr::OcrEngine,
    Storage::Streams::DataWriter,
    core::HSTRING,
};

pub fn recognize_rgba(width: u32, height: u32, rgba: &[u8]) -> Option<String> {
    let expected = usize::try_from(width)
        .ok()?
        .checked_mul(usize::try_from(height).ok()?)?
        .checked_mul(4)?;
    if width == 0 || height == 0 || rgba.len() < expected {
        return None;
    }
    let writer = DataWriter::new().ok()?;
    writer.WriteBytes(&rgba[..expected]).ok()?;
    let buffer = writer.DetachBuffer().ok()?;
    let bitmap = SoftwareBitmap::CreateCopyWithAlphaFromBuffer(
        &buffer,
        BitmapPixelFormat::Rgba8,
        i32::try_from(width).ok()?,
        i32::try_from(height).ok()?,
        BitmapAlphaMode::Straight,
    )
    .ok()?;
    let mut engines = Vec::new();
    if let Ok(engine) = OcrEngine::TryCreateFromUserProfileLanguages() {
        engines.push(engine);
    }
    for tag in ["vi", "en-US", "en"] {
        if let Ok(language) = Language::CreateLanguage(&HSTRING::from(tag))
            && OcrEngine::IsLanguageSupported(&language).unwrap_or(false)
            && let Ok(engine) = OcrEngine::TryCreateFromLanguage(&language)
        {
            let tag_str = language.LanguageTag().ok();
            if !engines.iter().any(|existing| {
                existing
                    .RecognizerLanguage()
                    .ok()
                    .and_then(|lang| lang.LanguageTag().ok())
                    == tag_str
            }) {
                engines.push(engine);
            }
        }
    }
    let mut combined = Vec::new();
    for engine in engines.into_iter().take(2) {
        if let Ok(operation) = engine.RecognizeAsync(&bitmap)
            && let Ok(result) = operation.get()
            && let Ok(lines) = result.Lines()
        {
            let mut chunk = Vec::new();
            let count = lines.Size().unwrap_or(0);
            for index in 0..count {
                if let Ok(line) = lines.GetAt(index)
                    && let Ok(text) = line.Text()
                {
                    let line_str = text.to_string_lossy();
                    let trimmed = line_str.trim();
                    if !trimmed.is_empty() {
                        chunk.push(trimmed.to_owned());
                    }
                }
            }
            let joined = chunk.join(" ");
            if !joined.is_empty() && !combined.contains(&joined) {
                combined.push(joined);
            }
        }
    }
    let output = combined.join("\n");
    (!output.trim().is_empty()).then_some(output)
}
