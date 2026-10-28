// SPDX-License-Identifier: MIT

//! Camera-container decoding is confined to the existing sandbox helper.

use flashraw::{
    Backend, DecodeOptions, Demosaic, FlashRaw, Limits, OutputFormat, PreviewEncoding, PreviewInfo,
    RawPixelData,
};
use gdk_pixbuf::prelude::*;
use image::ImageDecoder;
use std::{
    fs,
    io::{Cursor, Read},
    path::Path,
    time::Instant,
};

const MAX_INPUT: u64 = 256 * 1024 * 1024;
const MAX_PREVIEW: u64 = 32 * 1024 * 1024;
const MAX_PREVIEW_PIXELS: u64 = 16_000_000;
const MAX_SENSOR_PIXELS: u64 = 32_000_000;

fn open(path: &Path) -> Result<FlashRaw<'static>, String> {
    if !crate::sandbox::raw_metadata::is_raw(path) {
        return Err("Not a camera RAW file".into());
    }
    let file = fs::File::open(path).map_err(|error| error.to_string())?;
    let before = file.metadata().map_err(|error| error.to_string())?;
    if !before.is_file() || before.len() > MAX_INPUT {
        return Err("RAW input exceeds the preview budget".into());
    }
    // Owned, bounded input avoids mmap faults if another process truncates the file.
    let mut bytes = Vec::new();
    file.take(MAX_INPUT + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_INPUT || bytes.len() as u64 != before.len() {
        return Err("RAW input changed while reading".into());
    }
    let limits = Limits::default()
        .with_max_dimension(32768)
        .with_max_samples(128_000_000);
    FlashRaw::open_with(bytes, limits).map_err(|error| error.to_string())
}

pub(super) fn choose_preview(previews: &[PreviewInfo], edge: u32) -> Option<&PreviewInfo> {
    previews
        .iter()
        .filter(|preview| {
            matches!(
                preview.encoding,
                PreviewEncoding::Jpeg | PreviewEncoding::Rgb8
            ) && preview.length > 0
                && preview.length <= MAX_PREVIEW
                && u64::from(preview.width) * u64::from(preview.height) <= MAX_PREVIEW_PIXELS
        })
        .min_by_key(|preview| {
            let longest = preview.width.max(preview.height);
            (
                longest < edge,
                if longest >= edge {
                    u64::from(longest)
                } else {
                    u64::MAX - u64::from(longest)
                },
                preview.length,
            )
        })
}

pub(super) fn render(path: &Path, edge: u32, develop: bool) -> Result<Vec<u8>, String> {
    let started = Instant::now();
    let raw = open(path)?;
    let edge = edge.clamp(16, 2560);
    if !develop && let Some(preview) = choose_preview(raw.previews(), edge) {
        let bytes = raw
            .preview_bytes(preview)
            .map_err(|error| error.to_string())?;
        let mut image = match preview.encoding {
            PreviewEncoding::Jpeg => jpeg(bytes, edge)?,
            PreviewEncoding::Rgb8 => {
                let expected = u64::from(preview.width) * u64::from(preview.height) * 3;
                if expected != bytes.len() as u64 {
                    return Err("Invalid RAW RGB preview stride".into());
                }
                let image =
                    image::RgbImage::from_raw(preview.width, preview.height, bytes.to_vec())
                        .ok_or("Invalid RAW RGB preview")?;
                shrink(image::DynamicImage::ImageRgb8(image), edge)
            }
            _ => return Err("Unsupported RAW preview encoding".into()),
        };
        image.apply_orientation(
            image::metadata::Orientation::from_exif(preview.orientation.exif() as u8)
                .unwrap_or(image::metadata::Orientation::NoTransforms),
        );
        tracing::debug!(
            provider = "flashraw-embedded",
            elapsed_micros = started.elapsed().as_micros(),
            width = image.width(),
            height = image.height(),
            "RAW first pixels decoded"
        );
        return Ok(envelope(image.into_rgba8()));
    }
    if edge <= 256 {
        return Err("RAW has no supported embedded thumbnail".into());
    }
    let frame = raw.frame_info();
    if u64::from(frame.width) * u64::from(frame.height) > MAX_SENSOR_PIXELS {
        return Err("RAW sensor development exceeds the preview budget".into());
    }
    let frame = raw
        .decode(DecodeOptions {
            backend: Backend::CpuSimd,
            output: OutputFormat::Rgba8,
            threads: Some(2),
            demosaic: Demosaic::HalfSize,
            limits: Limits::default()
                .with_max_dimension(16384)
                .with_max_samples(MAX_SENSOR_PIXELS * 4),
            ..Default::default()
        })
        .map_err(|error| error.to_string())?;
    if !frame.warnings.is_empty() {
        return Err("RAW development reported damaged image data".into());
    }
    let RawPixelData::U8(pixels) = frame.data else {
        return Err("RAW decoder did not return RGBA8".into());
    };
    let image = image::RgbaImage::from_raw(frame.width, frame.height, pixels)
        .ok_or("Invalid RAW sensor output")?;
    let image = shrink(image::DynamicImage::ImageRgba8(image), edge).into_rgba8();
    tracing::debug!(
        provider = "flashraw-cpu",
        elapsed_micros = started.elapsed().as_micros(),
        "RAW sensor preview decoded"
    );
    Ok(envelope(image))
}

fn shrink(image: image::DynamicImage, edge: u32) -> image::DynamicImage {
    if image.width().max(image.height()) > edge {
        image.thumbnail(edge, edge)
    } else {
        image
    }
}

fn jpeg(bytes: &[u8], edge: u32) -> Result<image::DynamicImage, String> {
    let decoder = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| error.to_string())?
        .into_decoder()
        .map_err(|error| error.to_string())?;
    let (width, height) = decoder.dimensions();
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > MAX_PREVIEW_PIXELS {
        return Err("RAW embedded JPEG exceeds the preview budget".into());
    }
    let loader = gdk_pixbuf::PixbufLoader::with_type("jpeg").map_err(|error| error.to_string())?;
    let ratio = (edge as f64 / width.max(height) as f64).min(1.0);
    loader.set_size(
        (width as f64 * ratio).round().max(1.0) as i32,
        (height as f64 * ratio).round().max(1.0) as i32,
    );
    loader.write(bytes).map_err(|error| error.to_string())?;
    loader.close().map_err(|error| error.to_string())?;
    let pixbuf = loader
        .pixbuf()
        .ok_or("RAW JPEG decoder returned no pixels")?;
    let width = pixbuf.width() as u32;
    let height = pixbuf.height() as u32;
    let channels = pixbuf.n_channels() as usize;
    let stride = pixbuf.rowstride() as usize;
    if !matches!(channels, 3 | 4) || stride < width as usize * channels {
        return Err("Invalid RAW JPEG stride".into());
    }
    let source = pixbuf.read_pixel_bytes();
    let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
    for index in 0..height as usize {
        let row = source
            .get(index * stride..index * stride + width as usize * channels)
            .ok_or("Truncated RAW JPEG pixels")?;
        for pixel in row.chunks_exact(channels).take(width as usize) {
            pixels.extend_from_slice(&pixel[..3]);
            pixels.push(if channels == 4 { pixel[3] } else { 255 });
        }
    }
    if let Some(profile) = pixbuf.option("icc-profile") {
        use base64::Engine;
        if profile.len() > 6 * 1024 * 1024 {
            return Err("RAW color profile exceeds the preview budget".into());
        }
        let profile = base64::engine::general_purpose::STANDARD
            .decode(profile.as_bytes())
            .map_err(|_| "Invalid embedded RAW color profile")?;
        if profile.len() > 4 * 1024 * 1024 {
            return Err("RAW color profile exceeds the preview budget".into());
        }
        let profile =
            moxcms::ColorProfile::new_from_slice(&profile).map_err(|error| error.to_string())?;
        let transform = profile
            .create_transform_8bit(
                moxcms::Layout::Rgba,
                &moxcms::ColorProfile::new_srgb(),
                moxcms::Layout::Rgba,
                moxcms::TransformOptions::default(),
            )
            .map_err(|error| error.to_string())?;
        let mut converted = vec![0; pixels.len()];
        transform
            .transform(&pixels, &mut converted)
            .map_err(|error| error.to_string())?;
        pixels = converted;
    }
    image::RgbaImage::from_raw(width, height, pixels)
        .map(image::DynamicImage::ImageRgba8)
        .ok_or_else(|| "Invalid RAW JPEG pixels".into())
}

pub(super) fn metadata(path: &Path) -> Result<crate::sandbox::raw_metadata::RawMetadata, String> {
    let raw = open(path)?;
    let info = raw.metadata();
    let dimensions = info.image_size().or(Some((info.width, info.height)));
    Ok(crate::sandbox::raw_metadata::RawMetadata {
        dimensions,
        camera: Some(
            format!("{} {}", info.canonical_make, info.canonical_model)
                .trim()
                .to_owned(),
        ),
        lens: info.lens.model.clone(),
        focal_length: info.exposure.focal_length.map(f64::from),
        shutter_speed: info.exposure.exposure_time,
        iso: info.exposure.iso.map(f64::from),
        gps: info
            .gps
            .as_ref()
            .and_then(|gps| Some((gps.latitude?, gps.longitude?))),
    })
}

pub(super) fn envelope(image: image::RgbaImage) -> Vec<u8> {
    let (width, height) = image.dimensions();
    let mut output = Vec::with_capacity(16 + image.as_raw().len());
    output.extend_from_slice(b"STRGBA01");
    output.extend_from_slice(&width.to_le_bytes());
    output.extend_from_slice(&height.to_le_bytes());
    output.extend_from_slice(image.as_raw());
    output
}

#[cfg(test)]
mod tests;
