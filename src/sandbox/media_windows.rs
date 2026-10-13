// SPDX-License-Identifier: MIT
//! Win32 media sessions require verified child-process containment. A bare
//! GStreamer decoder in the main GTK process is not a secure substitute.
use crate::{
    media::{Header, Packet},
    services::SandboxedMedia,
};

pub(crate) const MAX_WORKERS: usize = 4;

pub(crate) enum Event {
    Prepared(Header),
    Packet(Packet),
    Failed(String),
}

/// API-compatible unavailable session. No GStreamer graph, audio device,
/// process or worker thread is started by this constructor.
pub(crate) struct Session;

impl Session {
    pub fn start(_source: SandboxedMedia, _start_tick: u32) -> Result<Self, String> {
        Err("Windows media decoding requires a native isolated helper".into())
    }
    pub fn receive(&self) -> Option<Event> {
        Some(Event::Failed("Windows media helper unavailable".into()))
    }
    pub fn cancel(&self) {}
    pub fn finished(&self) -> bool {
        true
    }
}

pub(crate) enum PeaksEvent {
    Levels { start: u32, levels: Vec<u8> },
    Finished,
    Failed,
}

pub(crate) struct PeaksSession;
impl PeaksSession {
    pub fn start(_source: SandboxedMedia) -> Option<Self> {
        None
    }
    pub fn receive(&self) -> Option<PeaksEvent> {
        Some(PeaksEvent::Failed)
    }
}

pub(crate) enum StoryboardEvent {
    Sheet(crate::media::storyboard::Sheet),
    Cell { index: u32, pixels: Vec<u8> },
    Finished,
    Failed,
}

pub(crate) struct StoryboardSession;
impl StoryboardSession {
    pub fn start(_source: SandboxedMedia, _cell_edge: u32) -> Option<Self> {
        None
    }
    pub fn receive(&self) -> Option<StoryboardEvent> {
        Some(StoryboardEvent::Failed)
    }
}
