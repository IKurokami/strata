// SPDX-License-Identifier: MIT
//! Win32 media sessions require verified child-process containment. A bare
//! GStreamer decoder in the main GTK process is not a secure substitute.
use crate::{
    media::{FPS, Frame, Header, MAX_DURATION_US, Packet},
    services::SandboxedMedia,
};
use std::{
    io::Read,
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
};

pub(crate) const MAX_WORKERS: usize = 4;

pub(crate) enum Event {
    Prepared(Header),
    Packet(Packet),
    Failed(String),
}

/// API-compatible unavailable session. No GStreamer graph, audio device,
/// process or worker thread is started by this constructor.
pub(crate) struct Session {
    cancelled: Arc<AtomicBool>,
    receiver: mpsc::Receiver<Event>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Session {
    pub fn start(source: SandboxedMedia, start_tick: u32) -> Result<Self, String> {
        let cancelled = Arc::new(AtomicBool::new(false));
        let stop = cancelled.clone();
        let (sender, receiver) = mpsc::sync_channel(3);
        let worker = thread::Builder::new()
            .name("windows-media-helper".into())
            .spawn(move || {
                if let Err(error) = decode_video(source, start_tick, &stop, &sender) {
                    let _ = sender.send(Event::Failed(error));
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            cancelled,
            receiver,
            worker: Some(worker),
        })
    }
    pub fn receive(&self) -> Option<Event> {
        self.receiver.try_recv().ok()
    }
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
    pub fn finished(&self) -> bool {
        self.worker
            .as_ref()
            .is_none_or(thread::JoinHandle::is_finished)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.cancel();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn decode_video(
    source: SandboxedMedia,
    start_tick: u32,
    cancelled: &AtomicBool,
    sender: &mpsc::SyncSender<Event>,
) -> Result<(), String> {
    if source.audio_only {
        return Err("Windows audio preview is not available yet".into());
    }
    let width = source.size.width.clamp(16, 1280) as u32;
    let height = source.size.height.clamp(16, 1280) as u32;
    let ffmpeg = std::env::var_os("STRATA_FFMPEG").unwrap_or_else(|| "ffmpeg".into());
    let mut child = Command::new(ffmpeg)
        .args(["-v", "error", "-ss", &format!("{:.3}", start_tick as f64 / FPS as f64), "-i"])
        .arg(&source.path)
        .args(["-vf", &format!("scale={width}:{height}:force_original_aspect_ratio=decrease,pad={width}:{height}:(ow-iw)/2:(oh-ih)/2,format=rgba"), "-f", "rawvideo", "-r", "30", "pipe:1"])
        .stdout(Stdio::piped()).stderr(Stdio::null()).spawn().map_err(|e| format!("Unable to start ffmpeg: {e}"))?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or("ffmpeg did not provide decoded output")?;
    let header = Header {
        width,
        height,
        audio: false,
        duration_us: MAX_DURATION_US,
        start_tick,
    };
    sender
        .send(Event::Prepared(header))
        .map_err(|_| "Preview closed".to_owned())?;
    let frame_bytes = header.video_bytes();
    let mut tick = start_tick;
    loop {
        if cancelled.load(Ordering::Acquire) {
            let _ = child.kill();
            return Ok(());
        }
        let mut pixels = vec![0; frame_bytes];
        let mut read = 0;
        while read < pixels.len() {
            let count = stdout
                .read(&mut pixels[read..])
                .map_err(|e| e.to_string())?;
            if count == 0 {
                break;
            }
            read += count;
        }
        if read == 0 {
            break;
        }
        pixels.truncate(read);
        sender
            .send(Event::Packet(Packet::Frame(Frame {
                tick,
                pixels,
                samples: Vec::new(),
            })))
            .map_err(|_| "Preview closed".to_owned())?;
        tick = tick.saturating_add(1);
    }
    let _ = child.wait();
    sender
        .send(Event::Packet(Packet::End(
            (tick as u64 * 1_000_000) / FPS as u64,
        )))
        .map_err(|_| "Preview closed".to_owned())?;
    Ok(())
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
