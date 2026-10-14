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

#[cfg(test)]
#[path = "media_windows/tests.rs"]
mod tests;

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
                    let _ = send(&sender, Event::Failed(error), &stop);
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
        // The decoder owns a blocking pipe read. Joining here would block the
        // GTK thread while a replaced selection is still draining FFmpeg.
        // Cancellation is observed by the worker and it tears down its child.
        self.worker.take();
    }
}

fn send(
    sender: &mpsc::SyncSender<Event>,
    mut event: Event,
    cancelled: &AtomicBool,
) -> Result<(), String> {
    loop {
        if cancelled.load(Ordering::Acquire) {
            return Err("Preview cancelled".into());
        }
        match sender.try_send(event) {
            Ok(()) => return Ok(()),
            Err(mpsc::TrySendError::Disconnected(_)) => return Err("Preview closed".into()),
            Err(mpsc::TrySendError::Full(pending)) => event = pending,
        }
        thread::sleep(std::time::Duration::from_millis(2));
    }
}

fn decode_video(
    source: SandboxedMedia,
    start_tick: u32,
    cancelled: &AtomicBool,
    sender: &mpsc::SyncSender<Event>,
) -> Result<(), String> {
    if source.audio_only {
        return decode_audio(source, start_tick, cancelled, sender);
    }
    let width = source.size.width.clamp(16, 1280) as u32;
    let height = source.size.height.clamp(16, 1280) as u32;
    let ffmpeg = media_tool("STRATA_FFMPEG", "ffmpeg.exe");
    let duration_us = probe_duration(&source.path).unwrap_or(MAX_DURATION_US);
    let mut child = Command::new(ffmpeg)
        .args(["-v", "error", "-i"])
        .arg(&source.path)
        .args(["-ss", &format!("{:.3}", start_tick as f64 / FPS as f64), "-vf", &format!("scale={width}:{height}:force_original_aspect_ratio=decrease,pad={width}:{height}:(ow-iw)/2:(oh-ih)/2,format=rgba"), "-f", "rawvideo", "-r", "30", "pipe:1"])
        .stdout(Stdio::piped()).stderr(Stdio::null()).spawn().map_err(|e| format!("Unable to start ffmpeg: {e}"))?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or("ffmpeg did not provide decoded output")?;
    let header = Header {
        width,
        height,
        audio: false,
        duration_us,
        start_tick,
    };
    let frame_bytes = header.video_bytes();
    send(sender, Event::Prepared(header), cancelled)?;
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
        if read != frame_bytes {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Incomplete decoded frame".into());
        }
        send(
            sender,
            Event::Packet(Packet::Frame(Frame {
                tick,
                pixels,
                samples: Vec::new(),
            })),
            cancelled,
        )
        .inspect_err(|_| {
            let _ = child.kill();
            let _ = child.wait();
        })?;
        tick = tick.saturating_add(1);
    }
    let _ = child.wait();
    send(
        sender,
        Event::Packet(Packet::End((tick as u64 * 1_000_000) / FPS as u64)),
        cancelled,
    )?;
    Ok(())
}

fn decode_audio(
    source: SandboxedMedia,
    start_tick: u32,
    cancelled: &AtomicBool,
    sender: &mpsc::SyncSender<Event>,
) -> Result<(), String> {
    let ffmpeg = media_tool("STRATA_FFMPEG", "ffmpeg.exe");
    let duration_us = probe_duration(&source.path).unwrap_or(MAX_DURATION_US);
    let mut child = Command::new(ffmpeg)
        .args(["-v", "error", "-i"])
        .arg(&source.path)
        // Accurate seek follows the input so restart does not land on an
        // earlier keyframe. PcmOutput expects interleaved signed 16-bit data.
        .args([
            "-ss",
            &format!("{:.3}", start_tick as f64 / FPS as f64),
            "-map",
            "0:a:0",
            "-vn",
            "-nostdin",
            "-f",
            "s16le",
            "-ac",
            "2",
            "-ar",
            "48000",
            "pipe:1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("Unable to start ffmpeg audio decoder: {error}"))?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or("ffmpeg did not provide audio output")?;
    let header = Header {
        width: 0,
        height: 0,
        audio: true,
        duration_us,
        start_tick,
    };
    send(sender, Event::Prepared(header), cancelled)?;
    let mut tick = start_tick;
    loop {
        if cancelled.load(Ordering::Acquire) {
            let _ = child.kill();
            return Ok(());
        }
        let chunk_bytes = if tick == start_tick {
            crate::media::AUDIO_BYTES * (crate::media::AUDIO_LEAD_TICKS as usize + 1)
        } else {
            crate::media::AUDIO_BYTES
        };
        let mut samples = vec![0; chunk_bytes];
        let mut read = 0;
        while read < samples.len() {
            let count = stdout
                .read(&mut samples[read..])
                .map_err(|error| error.to_string())?;
            if count == 0 {
                break;
            }
            read += count;
        }
        if read == 0 {
            break;
        }
        samples.truncate(read);
        send(
            sender,
            Event::Packet(Packet::Frame(Frame {
                tick,
                pixels: Vec::new(),
                samples,
            })),
            cancelled,
        )?;
        tick = tick.saturating_add(1);
    }
    let _ = child.wait();
    send(
        sender,
        Event::Packet(Packet::End((tick as u64 * 1_000_000) / FPS as u64)),
        cancelled,
    )
}

fn media_tool(override_name: &str, executable: &str) -> std::ffi::OsString {
    if let Some(value) = std::env::var_os(override_name) {
        return value;
    }
    if let Some(tool) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(executable)))
        .filter(|tool| tool.is_file())
    {
        return tool.into_os_string();
    }
    executable.into()
}

fn probe_duration(path: &std::path::Path) -> Option<u64> {
    let ffprobe = media_tool("STRATA_FFPROBE", "ffprobe.exe");
    let output = Command::new(ffprobe)
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(path)
        .output()
        .ok()?;
    let seconds = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<f64>()
        .ok()?;
    (seconds.is_finite() && seconds > 0.0)
        .then_some((seconds * 1_000_000.0).min(MAX_DURATION_US as f64) as u64)
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
