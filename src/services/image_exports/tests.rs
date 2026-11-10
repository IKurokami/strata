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
fn windows_exports_publish_valid_pngs_from_the_contained_helper() {
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
            &cancel,
        )
        .unwrap();
    let converted = webp.with_extension("png");
    assert_eq!(
        image::open(&converted).unwrap().to_rgba8().get_pixel(0, 0),
        &foreground
    );

    let paths = [converted, blue];
    for (name, layout) in [
        (DEFAULT_SHEET_NAME, SheetLayout::Grid),
        ("photos row.png", SheetLayout::Row),
        ("photos column.png", SheetLayout::Column),
    ] {
        Export::ContactSheet
            .run(&paths, name, layout, &cancel)
            .unwrap();
        let output = directory.path().join(name);
        let bytes = fs::read(&output).unwrap();
        let sheet = image::load_from_memory(&bytes).unwrap().into_rgba8();
        assert!(sheet.pixels().any(|pixel| *pixel == foreground));
        assert!(sheet.pixels().any(|pixel| *pixel == background));
        assert!(sheet.pixels().any(|pixel| pixel[3] == 0));
        assert!(matches!(
            Export::ContactSheet.run(&paths, name, layout, &cancel),
            Err(ExportError::Exists)
        ));
        assert_eq!(fs::read(output).unwrap(), bytes);
    }
    assert_eq!(fs::read(webp).unwrap(), original);
}
