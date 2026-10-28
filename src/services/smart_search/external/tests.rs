// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn pdf_and_media_locators_convert_units_without_shell_interpolation() {
    let path = Path::new("/docs/a file & data.pdf");
    let arguments = arguments(
        Path::new("SumatraPDF.exe"),
        path,
        &ContentLocator::Pdf {
            page_0: 4,
            start_char: 0,
            end_char: 0,
            anchor: String::new(),
        },
    )
    .expect("supported reader");
    assert_eq!(
        arguments,
        [
            OsString::from("-page"),
            OsString::from("5"),
            path.as_os_str().to_owned()
        ]
    );
    let media = super::arguments(
        Path::new("vlc"),
        path,
        &ContentLocator::Audio {
            start_us: 25_500_000,
            end_us: 55_500_000,
        },
    )
    .expect("supported player");
    assert_eq!(media[0], "--start-time=25.500");
    assert!(super::arguments(Path::new("unknown"), path, &ContentLocator::File).is_none());
}
