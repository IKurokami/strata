// SPDX-License-Identifier: MIT

use std::{
    collections::HashSet,
    fs,
    io::{self, Cursor, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};

use crate::sandbox::{self, Cancellation, MediaPreviewBackend, ParseOperation};
use image::{ImageFormat, ImageReader, Limits, RgbaImage, imageops};

pub(crate) const CELL: u32 = 256;
const GAP: u32 = 8;
pub(crate) const MAX_EXPORT_IMAGES: usize = 20;
static EXPORT: Mutex<()> = Mutex::new(());

#[derive(Clone, Copy)]
pub(crate) enum Export {
    Png,
    WebP,
    ContactSheet,
}

#[derive(Clone, Copy)]
pub(crate) enum SheetLayout {
    Grid { columns: u32 },
    Row,
    Column,
}

impl Default for SheetLayout {
    fn default() -> Self {
        Self::Grid { columns: 4 }
    }
}

impl SheetLayout {
    pub(crate) fn dimensions(self, count: usize) -> (u32, u32) {
        let count = count as u32;
        let columns = match self {
            Self::Grid { columns } => columns.clamp(1, count.max(1)),
            Self::Row => count.max(1),
            Self::Column => 1,
        };
        (columns, count.div_ceil(columns))
    }
}

pub(crate) const DEFAULT_SHEET_NAME: &str = "contact-sheet.png";

#[derive(Debug)]
pub(crate) enum ExportError {
    Unavailable,
    Exists,
    Busy,
    Cancelled,
}

impl Export {
    pub(crate) fn destination(self, paths: &[PathBuf], sheet_name: &str) -> Option<PathBuf> {
        let first = paths.first()?;
        let parent = first.parent()?;
        match self {
            Self::Png if paths.len() == 1 => Some(first.with_extension("png")),
            Self::WebP if paths.len() == 1 => Some(first.with_extension("webp")),
            Self::ContactSheet
                if (2..=MAX_EXPORT_IMAGES).contains(&paths.len())
                    && paths.iter().all(|path| path.parent() == Some(parent))
                    && valid_sheet_name(sheet_name) =>
            {
                Some(parent.join(sheet_name))
            }
            _ => None,
        }
    }

    pub(crate) fn run(
        self,
        paths: &[PathBuf],
        sheet_name: &str,
        sheet_layout: SheetLayout,
        cancel: &Cancellation,
    ) -> Result<(), ExportError> {
        let _slot = EXPORT.try_lock().map_err(|_| ExportError::Busy)?;
        self.run_one(paths, sheet_name, sheet_layout, cancel)
    }

    pub(crate) fn run_batch(
        self,
        paths: &[PathBuf],
        cancel: &Cancellation,
    ) -> Result<(), (usize, ExportError)> {
        if matches!(self, Self::ContactSheet) || !(2..=MAX_EXPORT_IMAGES).contains(&paths.len()) {
            return Err((0, ExportError::Unavailable));
        }
        let _slot = EXPORT.try_lock().map_err(|_| (0, ExportError::Busy))?;
        let mut destinations = HashSet::new();
        for path in paths {
            validate_input(path).map_err(|error| (0, error))?;
            let destination = self
                .destination(std::slice::from_ref(path), DEFAULT_SHEET_NAME)
                .ok_or((0, ExportError::Unavailable))?;
            if !destinations.insert(destination.clone()) {
                return Err((0, ExportError::Exists));
            }
            require_unused_destination(&destination).map_err(|error| (0, error))?;
        }
        for (completed, path) in paths.iter().enumerate() {
            self.run_one(
                std::slice::from_ref(path),
                DEFAULT_SHEET_NAME,
                SheetLayout::default(),
                cancel,
            )
            .map_err(|error| (completed, error))?;
        }
        Ok(())
    }

    fn run_one(
        self,
        paths: &[PathBuf],
        sheet_name: &str,
        sheet_layout: SheetLayout,
        cancel: &Cancellation,
    ) -> Result<(), ExportError> {
        let destination = self
            .destination(paths, sheet_name)
            .ok_or(ExportError::Unavailable)?;
        for path in paths {
            validate_input(path)?;
        }
        require_unused_destination(&destination)?;
        let parent = destination.parent().ok_or(ExportError::Unavailable)?;
        let mut output =
            tempfile::NamedTempFile::new_in(parent).map_err(|_| ExportError::Unavailable)?;
        match self {
            Self::Png | Self::WebP => {
                let operation = if matches!(self, Self::WebP) {
                    ParseOperation::ConvertWebP
                } else {
                    ParseOperation::ConvertImage
                };
                output
                    .write_all(&render(&paths[0], operation, 0, cancel)?)
                    .map_err(|_| ExportError::Unavailable)?;
            }
            Self::ContactSheet => {
                let sheet = contact_sheet(paths.len(), sheet_layout, |index| {
                    thumbnail(&paths[index], cancel)
                })?;
                image::DynamicImage::ImageRgba8(sheet)
                    .write_to(&mut output, ImageFormat::Png)
                    .map_err(|_| ExportError::Unavailable)?;
            }
        }
        if cancel.is_cancelled() {
            return Err(ExportError::Cancelled);
        }
        output.persist_noclobber(destination).map_err(|error| {
            if error.error.kind() == io::ErrorKind::AlreadyExists {
                ExportError::Exists
            } else {
                ExportError::Unavailable
            }
        })?;
        Ok(())
    }
}

fn require_unused_destination(path: &Path) -> Result<(), ExportError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(ExportError::Exists),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(ExportError::Unavailable),
    }
}

fn valid_sheet_name(name: &str) -> bool {
    if super::validate_basename(name).is_err() || name.len() > 255 {
        return false;
    }
    #[cfg(windows)]
    if !strata_native_io::windows_paths::valid_child_name(name.as_ref()) {
        return false;
    }
    Path::new(name)
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .is_some_and(|extension| extension.eq_ignore_ascii_case("png"))
}

fn validate_input(path: &Path) -> Result<(), ExportError> {
    if !path.is_absolute() {
        return Err(ExportError::Unavailable);
    }
    #[cfg(windows)]
    if !strata_win_shell::is_fixed_local_drive(path) {
        return Err(ExportError::Unavailable);
    }
    let metadata = fs::symlink_metadata(path).map_err(|_| ExportError::Unavailable)?;
    if !metadata.is_file() || metadata.len() > super::image_conversion::MAX_INPUT_BYTES {
        return Err(ExportError::Unavailable);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & (0x400 | 0x1000 | 0x40000 | 0x400000) != 0 {
            return Err(ExportError::Unavailable);
        }
    }
    Ok(())
}

fn render(
    path: &Path,
    operation: ParseOperation,
    size: i32,
    cancel: &Cancellation,
) -> Result<Vec<u8>, ExportError> {
    if cancel.is_cancelled() {
        return Err(ExportError::Cancelled);
    }
    sandbox::parse(path, operation, size, MediaPreviewBackend::Software, cancel)
        .map(|output| output.data)
        .map_err(|_| {
            if cancel.is_cancelled() {
                ExportError::Cancelled
            } else {
                ExportError::Unavailable
            }
        })
}

fn thumbnail(path: &Path, cancel: &Cancellation) -> Result<RgbaImage, ExportError> {
    let bytes = render(path, ParseOperation::ConvertImage, 1, cancel)?;
    let mut reader = ImageReader::with_format(Cursor::new(bytes), ImageFormat::Png);
    let mut limits = Limits::default();
    limits.max_image_width = Some(CELL);
    limits.max_image_height = Some(CELL);
    limits.max_alloc = Some(2 * 1024 * 1024);
    reader.limits(limits);
    reader
        .decode()
        .map(|image| image.into_rgba8())
        .map_err(|_| ExportError::Unavailable)
}

fn contact_sheet(
    count: usize,
    layout: SheetLayout,
    mut thumbnail: impl FnMut(usize) -> Result<RgbaImage, ExportError>,
) -> Result<RgbaImage, ExportError> {
    let (columns, rows) = layout.dimensions(count);
    let mut sheet = RgbaImage::new(columns * (CELL + GAP) + GAP, rows * (CELL + GAP) + GAP);
    for index in 0..count {
        let tile = thumbnail(index)?;
        let x = GAP + index as u32 % columns * (CELL + GAP) + (CELL - tile.width()) / 2;
        let y = GAP + index as u32 / columns * (CELL + GAP) + (CELL - tile.height()) / 2;
        imageops::replace(&mut sheet, &tile, i64::from(x), i64::from(y));
    }
    Ok(sheet)
}

#[cfg(test)]
mod tests;
