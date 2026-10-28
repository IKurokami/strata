// SPDX-License-Identifier: MIT

use super::*;
use flashraw::{Orientation, PreviewKind};
mod nef_fixture;

fn preview(edge: u32, encoding: PreviewEncoding) -> PreviewInfo {
    PreviewInfo::new(
        0,
        1024,
        edge,
        edge / 2,
        encoding,
        Orientation::Normal,
        PreviewKind::Thumbnail,
    )
}

#[test]
fn embedded_candidate_uses_smallest_sufficient_or_largest_available() {
    let candidates = [
        preview(160, PreviewEncoding::Jpeg),
        preview(1600, PreviewEncoding::Rgb8),
        preview(640, PreviewEncoding::Jpeg),
    ];
    for (edge, wanted) in [(256, 640), (1280, 1600), (2560, 1600)] {
        assert_eq!(
            choose_preview(&candidates, edge).expect("candidate").width,
            wanted
        );
    }
    let mut oversized = preview(256, PreviewEncoding::Jpeg);
    oversized.length = MAX_PREVIEW + 1;
    assert!(choose_preview(&[oversized], 256).is_none());
}

#[test]
fn raw_extension_cannot_bypass_container_validation() {
    let directory = tempfile::tempdir().expect("fixture");
    let path = directory.path().join("fake.dng");
    fs::write(&path, b"not a RAW container").expect("write");
    assert!(render(&path, 256, false).is_err());
}

#[test]
fn jpeg_stride_and_pixel_transport_preserve_all_rows() {
    let image = image::RgbImage::from_pixel(3, 2, image::Rgb([40, 80, 120]));
    let mut bytes = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(image)
        .write_to(&mut bytes, image::ImageFormat::Jpeg)
        .expect("fixture JPEG");
    let decoded = jpeg(bytes.get_ref(), 256).expect("decode JPEG");
    assert_eq!((decoded.width(), decoded.height()), (3, 2));
    let envelope = envelope(decoded.into_rgba8());
    assert_eq!(crate::sandbox::pixels::dimensions(&envelope), Some((3, 2)));
}

#[test]
fn embedded_nef_orientation_is_applied_once_without_sensor_decode() {
    let directory = tempfile::tempdir().expect("fixture");
    let path = directory.path().join("fixture.nef");
    for (orientation, expected) in [(1, (80, 40)), (6, (40, 80)), (8, (40, 80))] {
        fs::write(&path, nef_fixture::high_efficiency_nef(80, 40, orientation))
            .expect("synthetic NEF");
        let payload =
            render(&path, 256, false).expect("embedded preview despite unsupported sensor");
        let (dimensions, pixels) = crate::sandbox::pixels::decode(payload).expect("RGBA transport");
        assert_eq!(dimensions, expected);
        assert_eq!(pixels.len(), expected.0 as usize * expected.1 as usize * 4);
        if orientation == 6 {
            assert!(pixels[0] < 10);
            assert!(pixels[((expected.1 - 1) * expected.0 * 4) as usize] > 240);
        }
        let info = metadata(&path).expect("native metadata");
        assert!(
            info.camera
                .as_deref()
                .is_some_and(|camera| camera.to_ascii_lowercase().contains("nikon"))
        );
    }
}
