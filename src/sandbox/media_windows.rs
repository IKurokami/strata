// SPDX-License-Identifier: MIT
//! Win32 media sessions require verified child-process containment. A bare
//! GStreamer decoder in the main GTK process is not a secure substitute.
use crate::{
    media::{FPS, Frame, Header, MAX_DURATION_US, Packet},
    services::SandboxedMedia,
};
use std::{
    io::Read,
    process::{Child, ChildStdout, Command, Stdio},
    sync::{
        Arc, Mutex,
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
    children: Arc<Mutex<Vec<Child>>>,
}

impl Session {
    pub fn start(source: SandboxedMedia, start_tick: u32) -> Result<Self, String> {
        let cancelled = Arc::new(AtomicBool::new(false));
        let stop = cancelled.clone();
        let children = Arc::new(Mutex::new(Vec::new()));
        let worker_children = children.clone();
        let (sender, receiver) = mpsc::sync_channel(3);
        let worker = thread::Builder::new()
            .name("windows-media-helper".into())
            .spawn(move || {
                let result = decode_video(source, start_tick, &stop, &sender, &worker_children);
                // Ensure ffmpeg cannot survive a finished, failed, or cancelled preview.
                terminate_children(&worker_children);
                if let Err(error) = result {
                    let _ = send(&sender, Event::Failed(error), &stop);
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            cancelled,
            receiver,
            worker: Some(worker),
            children,
        })
    }
    pub fn receive(&self) -> Option<Event> {
        self.receiver.try_recv().ok()
    }
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        // Avoid waiting for a decoder process from the GTK event loop.
        kill_children(&self.children);
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

fn kill_children(children: &Arc<Mutex<Vec<Child>>>) {
    if let Ok(mut jobs) = children.lock() {
        for child in jobs.iter_mut() {
            let _ = child.kill();
        }
    }
}

fn terminate_children(children: &Arc<Mutex<Vec<Child>>>) {
    // Called from the worker, never the GTK event loop: reap all subprocesses.
    if let Ok(mut jobs) = children.lock() {
        for child in jobs.iter_mut() {
            let _ = child.kill();
        }
        for child in jobs.iter_mut() {
            let _ = child.wait();
        }
        jobs.clear();
    }
}

fn spawn_decoder(
    command: &mut Command,
    cancelled: &AtomicBool,
    children: &Arc<Mutex<Vec<Child>>>,
) -> Result<ChildStdout, String> {
    if cancelled.load(Ordering::Acquire) {
        return Err("Preview cancelled".into());
    }
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("Unable to start bundled media decoder: {error}"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or("Media decoder did not open a pipe")?;
    children
        .lock()
        .map_err(|_| "Media decoder process lock poisoned".to_owned())?
        .push(child);
    if cancelled.load(Ordering::Acquire) {
        terminate_children(children);
        return Err("Preview cancelled".into());
    }
    Ok(stdout)
}

fn read_chunk(reader: &mut impl Read, count: usize) -> Result<Vec<u8>, String> {
    let mut bytes = vec![0u8; count];
    let mut read = 0;
    while read < count {
        match reader.read(&mut bytes[read..]) {
            Ok(0) => break,
            Ok(n) => read += n,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(format!("Media decoder pipe failed: {error}")),
        }
    }
    bytes.truncate(read);
    Ok(bytes)
}

fn audio_command(source: &SandboxedMedia, start_tick: u32) -> Command {
    let mut cmd = Command::new(media_tool("STRATA_FFMPEG", "ffmpeg.exe"));
    cmd.arg("-v")
        .arg("error")
        // Input-side seek avoids decoding every preceding second on a scrub.
        .arg("-ss")
        .arg(format!("{:.3}", f64::from(start_tick) / f64::from(FPS)))
        .arg("-i")
        .arg(&source.path)
        .args([
            "-map",
            "0:a:0",
            "-vn",
            "-c:a",
            "pcm_s16le",
            "-f",
            "s16le",
            "-ac",
            "2",
            "-ar",
            "48000",
            "pipe:1",
        ]);
    cmd
}

fn video_command(source: &SandboxedMedia, start_tick: u32, width: u32, height: u32) -> Command {
    let mut cmd = Command::new(media_tool("STRATA_FFMPEG", "ffmpeg.exe"));
    cmd.arg("-v")
        .arg("error")
        .arg("-ss")
        .arg(format!("{:.3}", f64::from(start_tick) / f64::from(FPS)))
        .arg("-i")
        .arg(&source.path)
        .args(["-map", "0:v:0", "-an"])
        .arg("-vf")
        // `fitted_video_size` has already chosen dimensions with the source
        // aspect ratio. Scale to those exact dimensions so rawvideo emits a
        // fixed-size frame without a black pad or a short final read.
        .arg(format!("scale={width}:{height},format=rgba"))
        .args(["-f", "rawvideo", "-r", "30", "pipe:1"]);
    cmd
}

fn probe_video_dimensions(path: &std::path::Path) -> Option<(u32, u32)> {
    let ffprobe = media_tool("STRATA_FFPROBE", "ffprobe.exe");
    let output = Command::new(ffprobe)
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height",
            "-of",
            "csv=p=0:s=x",
        ])
        .arg(path)
        .output()
        .ok()?;
    let dimensions = String::from_utf8_lossy(&output.stdout);
    let (width, height) = dimensions.trim().split_once('x')?;
    let width = width.parse::<u32>().ok()?;
    let height = height.parse::<u32>().ok()?;
    (width > 0 && height > 0).then_some((width, height))
}

fn fitted_video_size(
    requested_width: u32,
    requested_height: u32,
    source_width: u32,
    source_height: u32,
) -> (u32, u32) {
    let requested_width = requested_width.max(16);
    let requested_height = requested_height.max(16);
    let source_width = source_width.max(1);
    let source_height = source_height.max(1);
    if u64::from(requested_width) * u64::from(source_height)
        <= u64::from(requested_height) * u64::from(source_width)
    {
        let height = (u64::from(requested_width) * u64::from(source_height)
            / u64::from(source_width))
        .max(2) as u32;
        (requested_width, height)
    } else {
        let width = (u64::from(requested_height) * u64::from(source_width)
            / u64::from(source_height))
        .max(2) as u32;
        (width, requested_height)
    }
}

fn has_audio(path: &std::path::Path) -> bool {
    Command::new(media_tool("STRATA_FFPROBE", "ffprobe.exe"))
        .args([
            "-v",
            "error",
            "-select_streams",
            "a:0",
            "-show_entries",
            "stream=index",
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .output()
        .is_ok_and(|result| result.status.success() && !result.stdout.is_empty())
}

fn decode_video(
    source: SandboxedMedia,
    start_tick: u32,
    cancelled: &AtomicBool,
    sender: &mpsc::SyncSender<Event>,
    children: &Arc<Mutex<Vec<Child>>>,
) -> Result<(), String> {
    if source.audio_only {
        return decode_audio(source, start_tick, cancelled, sender, children);
    }
    let requested_width = source.size.width.clamp(16, 1280) as u32;
    let requested_height = source.size.height.clamp(16, 1280) as u32;
    let (source_width, source_height) =
        probe_video_dimensions(&source.path).unwrap_or((requested_width, requested_height));
    let (width, height) = fitted_video_size(
        requested_width,
        requested_height,
        source_width,
        source_height,
    );
    let duration_us = probe_duration(&source.path).unwrap_or(MAX_DURATION_US);
    let audio = has_audio(&source.path);
    let mut pixels_pipe = spawn_decoder(
        &mut video_command(&source, start_tick, width, height),
        cancelled,
        children,
    )?;
    // Use separate bounded raw PCM and raw RGBA streams. They are decoded only
    // once each, never PNG/JPEG-encoded and never multiplexed through the GTK UI.
    let mut audio_pipe = if audio {
        Some(spawn_decoder(
            &mut audio_command(&source, start_tick),
            cancelled,
            children,
        )?)
    } else {
        None
    };
    let header = Header {
        width,
        height,
        audio,
        duration_us,
        start_tick,
    };
    let frame_bytes = header.video_bytes();
    send(sender, Event::Prepared(header), cancelled)?;
    let mut tick = start_tick;
    loop {
        if cancelled.load(Ordering::Acquire) {
            return Ok(());
        }
        let pixels = read_chunk(&mut pixels_pipe, frame_bytes)?;
        if pixels.is_empty() {
            break;
        }
        if pixels.len() != frame_bytes {
            return Err("Incomplete decoded video frame".into());
        }
        let samples = if let Some(pipe) = audio_pipe.as_mut() {
            let count = crate::media::audio_bytes(header, tick);
            let mut pcm = read_chunk(pipe, count)?;
            // Silence-pad a missing/short track instead of blocking the video
            // and keep the wire protocol's exact, contiguous PCM frame sizes.
            pcm.resize(count, 0);
            pcm
        } else {
            Vec::new()
        };
        send(
            sender,
            Event::Packet(Packet::Frame(Frame {
                tick,
                pixels,
                samples,
            })),
            cancelled,
        )?;
        tick = tick.saturating_add(1);
    }
    if tick == start_tick {
        return Err("Video decoder returned no frames".into());
    }
    send(
        sender,
        Event::Packet(Packet::End(if duration_us == MAX_DURATION_US {
            crate::media::timestamp(tick)
        } else {
            duration_us.min(crate::media::timestamp(tick))
        })),
        cancelled,
    )
}

fn decode_audio(
    source: SandboxedMedia,
    start_tick: u32,
    cancelled: &AtomicBool,
    sender: &mpsc::SyncSender<Event>,
    children: &Arc<Mutex<Vec<Child>>>,
) -> Result<(), String> {
    let duration_us = probe_duration(&source.path).unwrap_or(MAX_DURATION_US);
    let mut pipe = spawn_decoder(&mut audio_command(&source, start_tick), cancelled, children)?;
    let header = Header {
        width: 0,
        height: 0,
        audio: true,
        duration_us,
        start_tick,
    };
    send(sender, Event::Prepared(header), cancelled)?;
    let mut tick = start_tick;
    let mut total_pcm_bytes = 0u64;
    loop {
        if cancelled.load(Ordering::Acquire) {
            return Ok(());
        }
        let wanted = crate::media::audio_bytes(header, tick);
        let mut samples = read_chunk(&mut pipe, wanted)?;
        if samples.is_empty() {
            break;
        }
        samples.truncate(samples.len() & !3);
        if samples.is_empty() {
            break;
        }
        total_pcm_bytes += samples.len() as u64;
        let at_end = samples.len() < wanted;
        if tick == start_tick {
            // The first packet must cover AUDIO_LEAD_TICKS even for a sub-2s
            // clip. The UI trims the padded PCM to the probed media duration.
            samples.resize(wanted, 0);
        }
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
        if at_end {
            break;
        }
    }
    if tick == start_tick {
        return Err("Audio decoder returned no PCM samples".into());
    }
    let decoded_end = crate::media::timestamp(start_tick)
        + total_pcm_bytes * 1_000_000 / (crate::media::SAMPLE_RATE * 4);
    send(
        sender,
        Event::Packet(Packet::End(if duration_us == MAX_DURATION_US {
            decoded_end
        } else {
            duration_us.min(decoded_end)
        })),
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
