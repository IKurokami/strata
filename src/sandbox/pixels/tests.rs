// SPDX-License-Identifier: MIT

use super::*;

fn payload(width: u32, height: u32, pixels: &[u8]) -> Vec<u8> {
    let mut bytes = b"STRGBA01".to_vec();
    bytes.extend_from_slice(&width.to_le_bytes());
    bytes.extend_from_slice(&height.to_le_bytes());
    bytes.extend_from_slice(pixels);
    bytes
}

#[test]
fn rgba_transport_rejects_truncation_overflow_and_trailing_bytes() {
    let valid = payload(2, 1, &[4; 8]);
    assert_eq!(dimensions(&valid), Some((2, 1)));
    assert_eq!(
        decode(valid.clone()).expect("valid payload"),
        ((2, 1), vec![4; 8])
    );
    for end in 0..valid.len() {
        assert!(dimensions(&valid[..end]).is_none());
    }
    for (width, height) in [(0, 1), (1, 0), (u32::MAX, u32::MAX), (2561, 1)] {
        assert!(dimensions(&payload(width, height, &[0; 4])).is_none());
    }
    let mut trailing = valid;
    trailing.push(0);
    assert!(dimensions(&trailing).is_none());
    assert!(dimensions(b"\x89PNG\r\n\x1a\n").is_none());
}
