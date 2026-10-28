// SPDX-License-Identifier: MIT

#[cfg(test)]
mod tests;

/// Reserve most of the map for discovering entries, while retaining size order.
/// Measured bytes and percentages remain independent of these display weights.
pub(crate) fn balanced(bytes: &[u64]) -> Vec<f64> {
    let count = bytes.iter().filter(|bytes| **bytes > 0).count();
    if count == 0 {
        return vec![0.0; bytes.len()];
    }
    let total: f64 = bytes.iter().map(|bytes| *bytes as f64).sum();
    let equal_share = 0.6 / count as f64;
    bytes
        .iter()
        .map(|bytes| {
            if *bytes == 0 {
                0.0
            } else {
                equal_share + 0.4 * (*bytes as f64 / total)
            }
        })
        .collect()
}
