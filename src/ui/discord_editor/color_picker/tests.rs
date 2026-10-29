// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn picker_coordinates_cover_the_rgb_gamut_and_retain_imported_colors() {
    for color in PALETTE.into_iter().chain([
        Color(0, 0, 0),
        Color(255, 255, 255),
        Color(245, 251, 255),
        Color(84, 183, 243),
        Color(12, 34, 56),
    ]) {
        let hsv = Hsv::from_color(color);
        assert_eq!(hsv.color(), color);
        assert!((0.0..=1.0).contains(&hsv.s));
        assert!((0.0..=1.0).contains(&hsv.v));
    }
    for (hue, color) in [
        (0.0, Color(255, 0, 0)),
        (1.0 / 3.0, Color(0, 255, 0)),
        (2.0 / 3.0, Color(0, 0, 255)),
    ] {
        assert_eq!(
            Hsv {
                h: hue,
                s: 1.0,
                v: 1.0
            }
            .color(),
            color
        );
        assert_eq!(
            Hsv {
                h: hue,
                s: 0.0,
                v: 1.0
            }
            .color(),
            Color(255, 255, 255)
        );
        assert_eq!(
            Hsv {
                h: hue,
                s: 1.0,
                v: 0.0
            }
            .color(),
            Color(0, 0, 0)
        );
    }
}
