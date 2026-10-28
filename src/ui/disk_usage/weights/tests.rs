// SPDX-License-Identifier: MIT

use super::balanced;

#[test]
fn a_dominant_folder_leaves_small_files_discoverable() {
    for small_files in [1, 2, 9, 299] {
        let mut bytes = vec![1; small_files + 1];
        bytes[0] = u64::MAX;
        let weights = balanced(&bytes);
        let reserved_share = 0.6 / bytes.len() as f64;
        assert!(weights[0] <= 0.4 + reserved_share);
        assert!(weights[1..].iter().all(|weight| *weight >= reserved_share));
        assert!(weights[0] > weights[1]);
        assert!((weights.iter().sum::<f64>() - 1.0).abs() < 1e-12);
    }
}

#[test]
fn balancing_preserves_order_equal_sizes_and_zero_byte_exclusion() {
    let weights = balanced(&[0, 1, 4, 4, 100, 1_000]);
    assert_eq!(weights[0], 0.0);
    assert!(weights[1] < weights[2]);
    assert_eq!(weights[2], weights[3]);
    assert!(weights[3] <= weights[4]);
    assert!(weights[4] < weights[5]);
    assert!(weights.iter().all(|weight| weight.is_finite()));
    assert!((weights.iter().sum::<f64>() - 1.0).abs() < 1e-12);
}

#[test]
fn empty_and_single_entry_scopes_do_not_invent_usage() {
    assert!(balanced(&[]).is_empty());
    assert_eq!(balanced(&[0, 0]), [0.0, 0.0]);
    assert_eq!(balanced(&[0, 42, 0]), [0.0, 1.0, 0.0]);
}

#[test]
fn display_weights_depend_on_relative_sizes_not_the_byte_unit() {
    assert_eq!(balanced(&[1, 2, 4]), balanced(&[1_024, 2_048, 4_096]));
}
