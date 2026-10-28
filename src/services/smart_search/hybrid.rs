// SPDX-License-Identifier: MIT

use super::{
    result::{MatchReason, SearchHit},
    store::VectorIndex,
};
use std::collections::{HashMap, HashSet};
use unicode_normalization::{UnicodeNormalization, char::is_combining_mark};

pub(super) fn folded(text: &str) -> String {
    text.nfkd()
        .filter(|character| !is_combining_mark(*character))
        .flat_map(char::to_lowercase)
        .map(|character| if character == 'đ' { 'd' } else { character })
        .collect()
}

struct LexicalQuery {
    query: String,
    terms: Vec<String>,
}

impl LexicalQuery {
    fn new(query: &str) -> Self {
        let query = folded(query.trim());
        let mut terms: Vec<_> = words(&query).take(32).map(str::to_owned).collect();
        terms.sort_unstable();
        terms.dedup();
        Self { query, terms }
    }

    fn exact_filename(&self, hit: &SearchHit) -> bool {
        hit.path
            .file_name()
            .is_some_and(|name| folded(&name.to_string_lossy()) == self.query)
    }

    fn coverage(&self, hit: &SearchHit) -> f32 {
        let terms = &self.terms;
        if terms.is_empty() {
            return 0.0;
        }
        let document = folded(&format!(
            "{} {}",
            hit.path.file_name().unwrap_or_default().to_string_lossy(),
            hit.excerpt
        ));
        let document: HashSet<_> = words(&document).collect();
        terms
            .iter()
            .filter(|term| document.contains(term.as_str()))
            .count() as f32
            / terms.len() as f32
    }
}

fn words(text: &str) -> impl Iterator<Item = &str> {
    text.split(|character: char| !character.is_alphanumeric() && character != '_')
        .filter(|word| !word.is_empty())
}

pub(super) fn keyword_score(query: &str, hit: &SearchHit) -> i64 {
    let query = LexicalQuery::new(query);
    let exact = query.exact_filename(hit);
    let code = hit.modality == "code";
    let score = (if code { 1.0 } else { 2.0 }) * query.coverage(hit).powf(1.5)
        + if exact { 10.0 } else { 0.0 };
    (score * 1_000_000.0) as i64
}

fn requested_modality(query: &str) -> Option<&'static str> {
    let query = folded(query.trim());
    // A leading media noun is a preference, never a filter. Whole-word matching
    // avoids interpreting Vietnamese "ảnh hưởng" as a request for a picture.
    if query.starts_with("anh huong") {
        return None;
    }
    for (modality, prefixes) in [
        (
            "image",
            &[
                "ảnh",
                "hình ảnh",
                "hình",
                "image",
                "images",
                "photo",
                "photos",
                "picture",
                "pictures",
                "bild",
                "bilder",
                "foto",
                "fotos",
                "imagen",
                "imágenes",
                "immagine",
                "immagini",
                "imagem",
                "imagens",
                "画像",
                "사진",
                "이미지",
                "изображение",
                "фото",
                "图片",
                "图像",
            ][..],
        ),
        (
            "video",
            &[
                "video",
                "videos",
                "vidéo",
                "clip",
                "phim",
                "動画",
                "동영상",
                "비디오",
                "видео",
                "视频",
            ][..],
        ),
        (
            "audio",
            &[
                "audio",
                "music",
                "song",
                "nhạc",
                "bài hát",
                "musique",
                "musik",
                "música",
                "musica",
                "音楽",
                "노래",
                "음악",
                "музыка",
                "音乐",
            ][..],
        ),
    ] {
        if prefixes.iter().any(|prefix| {
            let prefix = folded(prefix);
            query.strip_prefix(&prefix).is_some_and(|rest| {
                rest.is_empty()
                    || rest.starts_with(char::is_whitespace)
                    || prefix.chars().any(|character| matches!(character, '\u{3040}'..='\u{30ff}' | '\u{4e00}'..='\u{9fff}'))
            })
        }) {
            return Some(modality);
        }
    }
    None
}

pub(super) fn distinct_files(mut hits: Vec<SearchHit>, limit: usize) -> Vec<SearchHit> {
    hits.sort_by(|left, right| {
        (
            left.weak,
            std::cmp::Reverse(left.score),
            &left.path,
            left.segment_id,
        )
            .cmp(&(
                right.weak,
                std::cmp::Reverse(right.score),
                &right.path,
                right.segment_id,
            ))
    });
    let mut seen = HashSet::new();
    hits.retain(|hit| seen.insert(hit.path.clone()));
    hits.truncate(limit);
    hits
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
        return distinct_files(keyword, 100);
    }
    let lexical = LexicalQuery::new(query);
    let requested = requested_modality(query);
    let mut statistics: HashMap<&str, (f64, f64, usize)> = HashMap::new();
    let mut candidates: HashMap<&str, Vec<usize>> = HashMap::new();
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
        candidates.entry(modality).or_default().push(position);
        scores.push(cosine);
    }
    let mut shortlisted = vec![false; scores.len()];
    for (modality, positions) in &mut candidates {
        positions.sort_unstable_by(|left, right| {
            scores[*right]
                .total_cmp(&scores[*left])
                .then_with(|| left.cmp(right))
        });
        let stat = statistics.entry(modality).or_default();
        let mut files = HashSet::new();
        for (visited, &position) in positions.iter().enumerate() {
            if visited % 128 == 0 && cancelled() {
                return Vec::new();
            }
            if !files.insert(&index.hits[position].path) {
                continue;
            }
            let cosine = f64::from(scores[position]);
            stat.0 += cosine;
            stat.1 += cosine * cosine;
            stat.2 += 1;
            if files.len() <= 256 {
                shortlisted[position] = true;
            }
        }
    }
    let keyword_segments: HashSet<_> = keyword
        .iter()
        .map(|hit| (hit.path.as_path(), hit.segment_id))
        .collect();
    for (position, hit) in index.hits.iter().enumerate() {
        if keyword_segments.contains(&(hit.path.as_path(), hit.segment_id)) {
            shortlisted[position] = true;
        }
    }
    let mut results: HashMap<_, _> = keyword
        .into_iter()
        .map(|mut hit| {
            let coverage = lexical.coverage(&hit);
            hit.score = ((coverage.powf(1.5) * 0.12
                + if lexical.exact_filename(&hit) {
                    10.0
                } else {
                    0.0
                })
                * 1_000_000.0) as i64;
            hit.weak = coverage < 0.75 && !lexical.exact_filename(&hit);
            hit
        })
        .map(|hit| (hit.identity(), hit))
        .collect();
    let exact_query = query.contains(['/', '\\', '.', ':']) || query.chars().any(char::is_numeric);
    for (position, cosine) in scores.into_iter().enumerate() {
        if position % 128 == 0 && cancelled() {
            return Vec::new();
        }
        if !shortlisted[position] {
            continue;
        }
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
        let word_coverage = lexical.coverage(source);
        let strong = word_coverage >= 0.75
            || (cosine >= 0.45 && (z >= 1.25 || cosine >= 0.62 || (count < 8 && z >= 0.5)));
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
        // Corpus statistics may only break close matches, not overturn a much
        // better cosine or reward a tiny unrelated modality with an outlier.
        let semantic_signal = cosine.max(0.0) + z.clamp(0.0, 2.0) * 0.04;
        let media_preference = if requested == Some(source.modality.as_str()) {
            0.18
        } else {
            0.0
        };
        let lexical_weight = if exact_query { 0.3 } else { 0.12 };
        hit.score = ((semantic_signal
            + lexical_weight * word_coverage.powf(1.5)
            + media_preference
            + if lexical.exact_filename(&hit) {
                10.0
            } else {
                0.0
            })
            * 1_000_000.0) as i64;
        results
            .entry(hit.identity())
            .and_modify(|old| {
                if (old.weak && !hit.weak) || (old.weak == hit.weak && hit.score > old.score) {
                    *old = hit.clone();
                }
            })
            .or_insert(hit);
    }
    distinct_files(results.into_values().collect(), 100)
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
