// SPDX-License-Identifier: MIT

use super::*;

static SERIAL: Mutex<()> = Mutex::new(());

#[test]
fn export_preserves_existing_output_and_cancelled_jobs_leave_no_files() {
    let _serial = SERIAL.lock().unwrap();
    let directory = tempfile::tempdir().expect("temporary directory");
    let source = directory.path().join("photo.jpg");
    let destination = directory.path().join("photo.png");
    fs::write(&source, b"original").unwrap();
    fs::write(&destination, b"existing output").unwrap();
    assert!(matches!(
        Export::Png.run(
            std::slice::from_ref(&source),
            DEFAULT_SHEET_NAME,
            SheetLayout::default(),
            false,
            &Cancellation::default()
        ),
        Err(ExportError::Exists)
    ));
    assert_eq!(fs::read(&destination).unwrap(), b"existing output");
    assert_eq!(fs::read(&source).unwrap(), b"original");

    fs::remove_file(destination).unwrap();
    let cancel = Cancellation::default();
    cancel.cancel();
    assert!(matches!(
        Export::Png.run(
            std::slice::from_ref(&source),
            DEFAULT_SHEET_NAME,
            SheetLayout::default(),
            false,
            &cancel
        ),
        Err(ExportError::Cancelled)
    ));
    assert_eq!(fs::read(&source).unwrap(), b"original");
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn sheet_names_must_be_png_basenames() {
    let paths = [PathBuf::from("folder/a.png"), PathBuf::from("folder/b.png")];
    for invalid in ["", ".png", "../escape.png", "nested/file.png", "sheet.jpg"] {
        assert!(Export::ContactSheet.destination(&paths, invalid).is_none());
    }
    #[cfg(windows)]
    for invalid in [
        r"..\escape.png",
        r"C:escape.png",
        "CON.png",
        "file.png:stream.png",
    ] {
        assert!(Export::ContactSheet.destination(&paths, invalid).is_none());
    }
    assert_eq!(
        Export::ContactSheet.destination(&paths, "Ảnh tổng hợp.PNG"),
        Some(PathBuf::from("folder").join("Ảnh tổng hợp.PNG"))
    );
}

#[cfg(windows)]
#[test]
fn windows_exports_publish_valid_images_from_the_contained_helper() {
    let _serial = SERIAL.lock().unwrap();
    let directory = tempfile::tempdir().expect("temporary directory");
    let webp = directory.path().join("photo.webp");
    let blue = directory.path().join("blue.png");
    let foreground = image::Rgba([185, 15, 28, 255]);
    let background = image::Rgba([22, 82, 160, 200]);
    RgbaImage::from_pixel(12, 12, foreground)
        .save(&webp)
        .unwrap();
    RgbaImage::from_pixel(12, 12, background)
        .save(&blue)
        .unwrap();
    let original = fs::read(&webp).unwrap();
    let cancel = Cancellation::default();

    Export::Png
        .run(
            std::slice::from_ref(&webp),
            DEFAULT_SHEET_NAME,
            SheetLayout::default(),
            false,
            &cancel,
        )
        .unwrap();
    let converted = webp.with_extension("png");
    assert_eq!(
        image::open(&converted).unwrap().to_rgba8().get_pixel(0, 0),
        &foreground
    );

    Export::WebP
        .run(
            std::slice::from_ref(&blue),
            DEFAULT_SHEET_NAME,
            SheetLayout::default(),
            false,
            &cancel,
        )
        .unwrap();
    let webp_output = blue.with_extension("webp");
    let bytes = fs::read(&webp_output).unwrap();
    assert!(crate::services::image_conversion::is_webp_output(&bytes));
    let decoded = image::load_from_memory(&bytes).unwrap().into_rgba8();
    assert!(decoded.pixels().all(|pixel| *pixel == background));
    assert!(matches!(
        Export::WebP.run(
            std::slice::from_ref(&blue),
            DEFAULT_SHEET_NAME,
            SheetLayout::default(),
            false,
            &cancel
        ),
        Err(ExportError::Exists)
    ));
    assert_eq!(fs::read(webp_output).unwrap(), bytes);

    let paths = [converted, blue];
    let transparent = directory.path().join("transparent.png");
    use image::{ImageDecoder, ImageEncoder};
    let mut encoder = image::codecs::png::PngEncoder::new(fs::File::create(&transparent).unwrap());
    encoder.set_icc_profile(b"test profile".to_vec()).unwrap();
    encoder
        .write_image(&[0; 12 * 12 * 4], 12, 12, image::ExtendedColorType::Rgba8)
        .unwrap();
    let jpeg_batch = [paths[0].clone(), transparent];
    Export::Jpeg.run_batch(&jpeg_batch, true, &cancel).unwrap();
    let white_jpeg = jpeg_batch[1].with_extension("jpg");
    let jpeg_bytes = fs::read(&white_jpeg).unwrap();
    assert!(crate::services::image_conversion::is_jpeg_output(
        &jpeg_bytes
    ));
    let mut jpeg = image::codecs::jpeg::JpegDecoder::new(Cursor::new(&jpeg_bytes)).unwrap();
    assert_eq!(jpeg.icc_profile().unwrap(), None);
    let decoded = image::load_from_memory(&jpeg_bytes).unwrap().into_rgb8();
    assert!(
        decoded
            .pixels()
            .all(|pixel| pixel.0.iter().all(|channel| *channel >= 250))
    );
    assert!(image::open(jpeg_batch[0].with_extension("jpg")).is_ok());
    assert!(matches!(
        Export::Jpeg.run_batch(&jpeg_batch, false, &cancel),
        Err((0, ExportError::Exists))
    ));
    assert_eq!(fs::read(white_jpeg).unwrap(), jpeg_bytes);

    let third = directory.path().join("third.png");
    let fourth = directory.path().join("fourth.png");
    fs::copy(&paths[0], &third).unwrap();
    fs::copy(&paths[1], &fourth).unwrap();
    Export::ContactSheet
        .run(
            &[paths[0].clone(), paths[1].clone(), third, fourth],
            "two-by-two.png",
            SheetLayout::Grid { columns: 2 },
            false,
            &cancel,
        )
        .unwrap();
    let square = image::open(directory.path().join("two-by-two.png")).unwrap();
    assert_eq!(square.width(), square.height());

    for (name, layout) in [
        (DEFAULT_SHEET_NAME, SheetLayout::default()),
        ("photos row.png", SheetLayout::Row),
        ("photos column.png", SheetLayout::Column),
    ] {
        Export::ContactSheet
            .run(&paths, name, layout, false, &cancel)
            .unwrap();
        let output = directory.path().join(name);
        let bytes = fs::read(&output).unwrap();
        let sheet = image::load_from_memory(&bytes).unwrap().into_rgba8();
        assert!(sheet.pixels().any(|pixel| *pixel == foreground));
        assert!(sheet.pixels().any(|pixel| *pixel == background));
        assert!(sheet.pixels().any(|pixel| pixel[3] == 0));
        assert!(matches!(
            Export::ContactSheet.run(&paths, name, layout, false, &cancel),
            Err(ExportError::Exists)
        ));
        assert_eq!(fs::read(output).unwrap(), bytes);
    }
    assert_eq!(fs::read(webp).unwrap(), original);

    let first = directory.path().join("batch-first.png");
    let second = directory.path().join("batch-second.png");
    fs::copy(&paths[1], &first).unwrap();
    fs::write(&second, b"invalid image").unwrap();
    let batch = [first, second];
    let first_output = batch[0].with_extension("webp");
    let second_output = batch[1].with_extension("webp");
    fs::write(&second_output, b"existing output").unwrap();
    assert!(matches!(
        Export::WebP.run_batch(&batch, false, &cancel),
        Err((0, ExportError::Exists))
    ));
    assert!(!first_output.exists());
    assert_eq!(fs::read(&second_output).unwrap(), b"existing output");
    fs::remove_file(second_output).unwrap();
    assert!(matches!(
        Export::WebP.run_batch(&batch, false, &cancel),
        Err((1, ExportError::Unavailable))
    ));
    assert!(image::open(first_output).is_ok());
    assert_eq!(fs::read(&batch[1]).unwrap(), b"invalid image");
}
