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
        Export::Png.run(std::slice::from_ref(&source), &Cancellation::default()),
        Err(ExportError::Exists)
    ));
    assert_eq!(fs::read(&destination).unwrap(), b"existing output");
    assert_eq!(fs::read(&source).unwrap(), b"original");

    fs::remove_file(destination).unwrap();
    let cancel = Cancellation::default();
    cancel.cancel();
    assert!(matches!(
        Export::Png.run(std::slice::from_ref(&source), &cancel),
        Err(ExportError::Cancelled)
    ));
    assert_eq!(fs::read(&source).unwrap(), b"original");
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
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
        .run(std::slice::from_ref(&webp), &cancel)
        .unwrap();
    let converted = webp.with_extension("png");
    assert_eq!(
        image::open(&converted).unwrap().to_rgba8().get_pixel(0, 0),
        &foreground
    );

    Export::ContactSheet
        .run(&[converted, blue], &cancel)
        .unwrap();
    let sheet = image::open(directory.path().join("contact-sheet.png"))
        .unwrap()
        .into_rgba8();
    assert!(sheet.pixels().any(|pixel| *pixel == foreground));
    assert!(sheet.pixels().any(|pixel| *pixel == background));
    assert!(sheet.pixels().any(|pixel| pixel[3] == 0));
    assert_eq!(fs::read(webp).unwrap(), original);
}
