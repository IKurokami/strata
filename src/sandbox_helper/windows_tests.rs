// SPDX-License-Identifier: MIT

use super::render_raw;

#[test]
fn background_probe_capture_preserves_output_and_failure_status() {
    use std::{process::Command, time::Duration};
    for timed in [false, true] {
        for code in [0, 7] {
            let script = format!(
                r#"Add-Type 'using System; using System.Runtime.InteropServices; public static class ConsoleProbe {{ [DllImport("kernel32.dll")] public static extern IntPtr GetConsoleWindow(); }}';
if ([ConsoleProbe]::GetConsoleWindow() -ne [IntPtr]::Zero) {{ exit 91 }}
[Console]::Out.Write('probe-output'); exit {code}"#
            );
            let mut command = Command::new("powershell.exe");
            command.args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                &script,
            ]);
            let output = if timed {
                super::bounded_output_with_timeout(&mut command, 1024, Duration::from_secs(15))
                    .expect("hidden probe")
                    .expect("probe completed")
            } else {
                super::bounded_output(&mut command, 1024).expect("hidden thumbnail provider")
            };
            assert_eq!(output.status.code(), Some(code), "timed={timed}");
            assert_eq!(
                String::from_utf8_lossy(&output.stdout).trim(),
                "probe-output",
                "timed={timed}"
            );
        }
    }
}

#[test]
fn image_fallback_decodes_webp_and_gif_on_windows() {
    let directory = tempfile::tempdir().expect("image fixture");
    let webp = directory.path().join("image.webp");
    image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
        80,
        40,
        image::Rgba([0x33, 0x66, 0x99, 0xff]),
    ))
    .save_with_format(&webp, image::ImageFormat::WebP)
    .expect("write WebP fixture");

    let gif = directory.path().join("image.gif");
    let mut bytes = Vec::new();
    image::codecs::gif::GifEncoder::new(&mut bytes)
        .encode_frame(image::Frame::new(image::RgbaImage::from_pixel(
            64,
            32,
            image::Rgba([0x99, 0x66, 0x33, 0xff]),
        )))
        .expect("write GIF fixture");
    std::fs::write(&gif, bytes).expect("save GIF fixture");

    for (path, aspect) in [(&webp, (2, 1)), (&gif, (2, 1))] {
        let png = render_raw(path, 800).expect("render image preview");
        let decoded = image::load_from_memory(&png).expect("decode PNG preview");
        assert!(
            decoded.width() <= 800 && decoded.height() <= 800,
            "{}",
            path.display()
        );
        assert_eq!(decoded.width() * aspect.1, decoded.height() * aspect.0);
    }
}
