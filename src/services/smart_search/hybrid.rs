// SPDX-License-Identifier: MIT

use super::{
    result::{MatchReason, SearchHit},
    store::VectorIndex,
};
use std::collections::HashMap;
use unicode_normalization::{UnicodeNormalization, char::is_combining_mark};

pub(super) fn folded(text: &str) -> String {
    text.nfkd()
        .filter(|character| !is_combining_mark(*character))
        .flat_map(char::to_lowercase)
        .map(|character| if character == 'đ' { 'd' } else { character })
        .collect()
}

pub(super) fn coverage(query: &str, hit: &SearchHit) -> f32 {
    let terms: Vec<_> = query.split_whitespace().take(32).map(folded).collect();
    if terms.is_empty() {
        return 0.0;
    }
    let document = folded(&format!(
        "{} {}",
        hit.path.file_name().unwrap_or_default().to_string_lossy(),
        hit.excerpt
    ));
    terms
        .iter()
        .filter(|term| document.contains(term.as_str()))
        .count() as f32
        / terms.len() as f32
}

pub(super) fn keyword_score(query: &str, hit: &SearchHit) -> i64 {
    let exact = hit
        .path
        .file_name()
        .is_some_and(|name| folded(&name.to_string_lossy()) == folded(query));
    let code = hit.modality == "code";
    let score = (if code { 1.0 } else { 2.0 }) * coverage(query, hit).powf(1.5)
        + if exact { 10.0 } else { 0.0 };
    (score * 1_000_000.0) as i64
}

pub(super) fn rank(
    query: &str,
    embedding: &[f32],
    code_embedding: Option<&[f32]>,
    index: &VectorIndex,
    keyword: Vec<SearchHit>,
    cancelled: impl Fn() -> bool,
) -> Vec<SearchHit> {
    if embedding.len() != 768 {
        return keyword;
    }
    let mut statistics: HashMap<&str, (f64, f64, usize)> = HashMap::new();
    let mut scores = Vec::with_capacity(index.hits.len());
    for (position, row) in index.values.as_chunks::<768>().0.iter().enumerate() {
        if position % 128 == 0 && cancelled() {
            return Vec::new();
        }
        let selected = if index.hits[position].modality == "code" {
            code_embedding
                .filter(|vector| vector.len() == 768)
                .unwrap_or(embedding)
        } else {
            embedding
        };
        let cosine: f32 = row
            .iter()
            .zip(selected)
            .map(|(left, right)| left.to_f32() * right)
            .sum();
        let modality = index.hits[position].modality.as_str();
        let stat = statistics.entry(modality).or_default();
        stat.0 += f64::from(cosine);
        stat.1 += f64::from(cosine).powi(2);
        stat.2 += 1;
        scores.push(cosine);
    }
    let mut results: HashMap<_, _> = keyword
        .into_iter()
        .map(|hit| (hit.identity(), hit))
        .collect();
    let exact_query = query.contains(['/', '\\', '.', ':']) || query.chars().any(char::is_numeric);
    for (position, cosine) in scores.into_iter().enumerate() {
        let source = &index.hits[position];
        let Some((sum, squares, count)) = statistics.get(source.modality.as_str()).copied() else {
            continue;
        };
        let mean = sum / count as f64;
        let deviation = (squares / count as f64 - mean * mean).max(0.0).sqrt();
        let reg_deviation = deviation.max(0.035);
        let z = if count > 1 && deviation > 1e-6 {
            ((f64::from(cosine) - mean) / reg_deviation) as f32
        } else {
            ((cosine - 0.35).max(0.0)) * 3.0
        };
        let word_coverage = coverage(query, source);
        let strong = z >= 1.25
            || cosine >= 0.62
            || (count < 8 && cosine >= 0.45 && z >= 0.5)
            || word_coverage >= 0.34;
        if !strong && cosine < 0.28 && word_coverage == 0.0 {
            continue;
        }
        let mut hit = source.clone();
        hit.reason = if word_coverage > 0.0 {
            MatchReason::Hybrid
        } else {
            MatchReason::Semantic
        };
        hit.weak = !strong;
        let semantic_weight = if exact_query { 0.5 } else { 1.0 };
        let keyword_weight = if exact_query { 2.0 } else { 1.0 };
        let semantic_signal = cosine.max(0.0) * 1.5 + z.max(0.0) * 0.6;
        hit.score = (keyword_score(query, &hit) as f32 * keyword_weight) as i64
            + (semantic_weight * semantic_signal * 1_000_000.0) as i64;
        results
            .entry(hit.identity())
            .and_modify(|old| {
                if hit.score > old.score {
                    *old = hit.clone();
                }
            })
            .or_insert(hit);
    }
    let mut results: Vec<_> = results.into_values().collect();
    results.sort_by_key(|hit| {
        (
            hit.weak,
            std::cmp::Reverse(hit.score),
            hit.file_id,
            hit.segment_id,
        )
    });
    results.truncate(100);
    results
}

#[cfg(test)]
mod tests;

pub(super) fn locate(query: &str, hit: &mut SearchHit) {
    use super::result::ContentLocator;
    let mut normalized = String::new();
    let mut offsets = Vec::new();
    for (index, character) in hit.excerpt.chars().enumerate() {
        for folded in folded(&character.to_string()).chars() {
            offsets.extend(std::iter::repeat_n(index, folded.len_utf8()));
            normalized.push(folded);
        }
    }
    let needle = folded(query);
    let found = normalized
        .find(&needle)
        .map(|start| (start, start + needle.len()))
        .or_else(|| {
            needle.split_whitespace().find_map(|term| {
                normalized
                    .find(term)
                    .map(|start| (start, start + term.len()))
            })
        });
    if let Some((start, end)) = found
        && let (Some(&start), Some(&last)) =
            (offsets.get(start), offsets.get(end.saturating_sub(1)))
    {
        let end = last + 1;
        let anchor: String = hit.excerpt.chars().skip(start).take(end - start).collect();
        match &mut hit.locator {
            ContentLocator::Pdf {
                start_char,
                end_char,
                anchor: target,
                ..
            }
            | ContentLocator::Text {
                start_char,
                end_char,
                anchor: target,
                ..
            } => {
                let base = *start_char;
                *start_char = base.saturating_add(start as u32);
                *end_char = base.saturating_add(end as u32);
                *target = anchor;
            }
            ContentLocator::Code {
                line_1,
                end_line_1,
                anchor: target,
            } => {
                let before = hit
                    .excerpt
                    .chars()
                    .take(start)
                    .filter(|character| *character == '\n')
                    .count() as u32;
                *line_1 = line_1.saturating_add(before);
                *end_line_1 = *line_1;
                *target = anchor;
            }
            _ => {}
        }
        hit.excerpt = hit
            .excerpt
            .chars()
            .skip(start.saturating_sub(70))
            .take(240)
            .collect();
    } else {
        hit.excerpt = hit.excerpt.chars().take(240).collect();
    }
}
