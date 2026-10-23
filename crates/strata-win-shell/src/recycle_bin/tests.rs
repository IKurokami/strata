// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn original_records_preserve_extensions_unicode_and_reject_malformed_paths() {
    let path = Path::new(r"C:\Users\tester\ảnh.txt.txt");
    let words = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let mut version2 = vec![0u8; 28];
    version2[..8].copy_from_slice(&2u64.to_le_bytes());
    version2[24..28].copy_from_slice(&(words.len() as u32).to_le_bytes());
    for word in words {
        version2.extend_from_slice(&word.to_le_bytes());
    }
    assert_eq!(decode_original_record(&version2).unwrap(), path);
    let mut version1 = vec![0; 544];
    version1[..8].copy_from_slice(&1u64.to_le_bytes());
    version1[24..24 + version2.len() - 28].copy_from_slice(&version2[28..]);
    assert_eq!(decode_original_record(&version1).unwrap(), path);
    for malformed in [
        vec![],
        version2[..version2.len() - 1].to_vec(),
        vec![0; 65_537],
    ] {
        assert!(decode_original_record(&malformed).is_err());
    }
    version2[24..28].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(decode_original_record(&version2).is_err());
    version1[24..].fill(0);
    assert!(decode_original_record(&version1).is_err());
}
