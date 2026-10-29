// SPDX-License-Identifier: MIT

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub(crate) enum ContentLocator {
    File,
    Pdf {
        page_0: u32,
        start_char: u32,
        end_char: u32,
        anchor: String,
    },
    Text {
        chunk: u32,
        start_char: u32,
        end_char: u32,
        anchor: String,
    },
    Code {
        line_1: u32,
        end_line_1: u32,
        anchor: String,
    },
    Audio {
        start_us: u64,
        end_us: u64,
    },
    Video {
        start_us: u64,
        end_us: u64,
        soundtrack: bool,
    },
    Slide {
        slide_0: u32,
        anchor: String,
    },
    Sheet {
        name: String,
        row_1: u32,
        col_1: u32,
        anchor: String,
    },
    Image,
}

impl ContentLocator {
    pub fn page(&self) -> Option<u32> {
        if let Self::Pdf { page_0, .. } = self {
            Some(*page_0)
        } else {
            None
        }
    }
    pub fn time_range(&self) -> Option<(u64, u64)> {
        match self {
            Self::Audio { start_us, end_us }
            | Self::Video {
                start_us, end_us, ..
            } => Some((*start_us, *end_us)),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub(crate) struct FileRevision {
    pub size: u64,
    pub modified_ns: u64,
}

impl FileRevision {
    pub fn from_metadata(metadata: &std::fs::Metadata) -> Self {
        let modified_ns = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |time| time.as_nanos().min(u128::from(u64::MAX)) as u64);
        Self {
            size: metadata.len(),
            modified_ns,
        }
    }
    pub fn key(&self) -> String {
        format!("{:x}:{:x}", self.size, self.modified_ns)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) enum MatchReason {
    Filename,
    Keyword,
    Semantic,
    Hybrid,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct SearchHit {
    pub file_id: i64,
    pub segment_id: i64,
    pub path: PathBuf,
    pub revision: FileRevision,
    pub locator: ContentLocator,
    pub excerpt: String,
    pub modality: String,
    pub reason: MatchReason,
    pub score: i64,
    pub weak: bool,
}

impl SearchHit {
    pub fn identity(&self) -> (PathBuf, i64) {
        (self.path.clone(), self.segment_id)
    }
    pub fn entry(&self) -> crate::model::FileEntry {
        use crate::model::{EntryKind, FileEntry, Location, MetadataValue};
        let name = self
            .path
            .file_name()
            .unwrap_or(self.path.as_os_str())
            .to_owned();
        FileEntry {
            location: Location::local(&self.path),
            thumbnail_path: None,
            native_name: name.clone(),
            display_name: name.to_string_lossy().into_owned(),
            kind: EntryKind::File,
            size: MetadataValue::Known(self.revision.size),
            modified_unix_seconds: MetadataValue::Known(
                (self.revision.modified_ns / 1_000_000_000) as i64,
            ),
            recent_unix_seconds: MetadataValue::Unknown,
            recent_uri: None,
            mode: MetadataValue::Unknown,
            image_dimensions: MetadataValue::Unknown,
            child_count: MetadataValue::Unavailable,
            duration_seconds: MetadataValue::Unknown,
            is_hidden: false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct ExtractedSegment {
    pub locator: ContentLocator,
    pub text: String,
    pub modality: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Extraction {
    pub segments: Vec<ExtractedSegment>,
    pub next: Option<u32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) enum SearchPhase {
    Keyword,
    Semantic,
}

#[derive(Clone, Debug)]
pub(crate) struct SearchResponse {
    pub generation: u64,
    pub configuration: u64,
    pub phase: SearchPhase,
    pub hits: Vec<SearchHit>,
}

pub(crate) fn accepts_phase(
    generation: u64,
    phase: Option<SearchPhase>,
    response: &SearchResponse,
) -> bool {
    generation == response.generation && phase.is_none_or(|current| response.phase >= current)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[expect(
    dead_code,
    reason = "The index retains category filters; the compact UI currently queries all kinds."
)]
pub(crate) enum SearchKind {
    Any,
    Documents,
    Images,
    Audio,
    Video,
    Code,
}
impl SearchKind {
    pub fn accepts(self, path: &std::path::Path) -> bool {
        match self {
            Self::Any => true,
            Self::Code => super::policy::code(path),
            Self::Images => super::policy::image(path),
            Self::Audio => super::policy::media(path) == Some(false),
            Self::Video => super::policy::media(path) == Some(true),
            Self::Documents => {
                !super::policy::code(path)
                    && (super::policy::plain(path)
                        || matches!(
                            super::policy::extension(path).as_str(),
                            "pdf" | "docx" | "odt" | "pptx" | "odp" | "xlsx" | "ods"
                        ))
            }
        }
    }
}
