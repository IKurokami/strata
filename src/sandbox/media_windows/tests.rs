// SPDX-License-Identifier: MIT
use super::*;
use std::time::{Duration, Instant};

#[test]
fn cancellation_does_not_wait_for_the_child_registry() {
    let children = Arc::new(Mutex::new(Vec::new()));
    let guard = children.lock().unwrap();
    let session = Session {
        cancelled: Arc::new(AtomicBool::new(false)),
        receiver: mpsc::channel().1,
        worker: None,
        children: children.clone(),
    };
    let (sender, receiver) = mpsc::channel();
    let caller = thread::spawn(move || {
        session.cancel();
        sender.send(()).unwrap();
    });
    let returned = receiver.recv_timeout(Duration::from_secs(1));
    drop(guard);
    caller.join().unwrap();
    assert!(returned.is_ok(), "cancellation waited for decoder teardown");
}

#[test]
fn windows_video_seek_resumes_at_the_requested_frame() {
    let (_dir, path) = generated_media(
        "mp4",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=128x72:rate=30",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=880:sample_rate=48000",
            "-t",
            "3",
            "-c:v",
            "mpeg4",
            "-c:a",
            "aac",
            "-shortest",
        ],
    );
    let source = test_source(path, false);
    let initial = Session::start(source.clone(), 0).unwrap();
    let (_, before) = first_frame(&initial);
    initial.cancel();
    let sought = Session::start(source, FPS * 2).unwrap();
    let (header, after) = first_frame(&sought);
    assert_eq!(header.start_tick, FPS * 2);
    assert_eq!(after.tick, FPS * 2);
    assert_ne!(before.pixels, after.pixels);
    assert!(header.audio);
    assert!(after.samples.iter().any(|byte| *byte != 0));
}

#[test]
fn cached_probe_is_invalidated_when_media_is_replaced() {
    let (_first_dir, path) = generated_media(
        "wav",
        &[
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000",
            "-t",
            "1",
        ],
    );
    let (_second_dir, replacement) = generated_media(
        "wav",
        &[
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=880:sample_rate=48000",
            "-t",
            "2",
        ],
    );
    assert_eq!(probe_duration(&path), Some(1_000_000));
    assert_eq!(probe_duration(&path), Some(1_000_000));
    std::fs::copy(replacement, &path).unwrap();
    assert_eq!(probe_duration(&path), Some(2_000_000));
}

#[test]
fn cancellation_releases_a_full_packet_queue() {
    let (sender, _receiver) = mpsc::sync_channel(1);
    sender
        .send(Event::Packet(Packet::End(1)))
        .expect("fill queue");
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = cancelled.clone();
    let worker = thread::spawn(move || send(&sender, Event::Packet(Packet::End(2)), &flag));
    thread::sleep(Duration::from_millis(10));
    cancelled.store(true, Ordering::Release);
    let deadline = Instant::now() + Duration::from_secs(1);
    while !worker.is_finished() {
        assert!(
            Instant::now() < deadline,
            "cancelled worker retained full queue"
        );
        thread::sleep(Duration::from_millis(1));
    }
    assert!(worker.join().expect("worker").is_err());
}

fn generated_media(extension: &str, args: &[&str]) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("temporary media fixture");
    let path = dir.path().join(format!("preview.{extension}"));
    let result = Command::new(media_tool("STRATA_FFMPEG", "ffmpeg.exe"))
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y"])
        .args(args)
        .arg(&path)
        .output()
        .expect("bundled ffmpeg must be available");
    assert!(
        result.status.success(),
        "fixture encoder failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    (dir, path)
}

fn test_source(path: std::path::PathBuf, audio_only: bool) -> SandboxedMedia {
    SandboxedMedia {
        path,
        size: crate::services::MediaPreviewSize::new(96, 64),
        backend: crate::sandbox::MediaPreviewBackend::Software,
        input_owner: None,
        audio_only,
    }
}

fn first_frame(session: &Session) -> (Header, Frame) {
    let until = Instant::now() + Duration::from_secs(12);
    let mut header = None;
    while Instant::now() < until {
        match session.receive() {
            Some(Event::Prepared(value)) => header = Some(value),
            Some(Event::Packet(Packet::Frame(frame))) => {
                return (header.expect("header before pixels/PCM"), frame);
            }
            Some(Event::Failed(error)) => panic!("decoder failure: {error}"),
            Some(Event::Packet(Packet::End(_))) => panic!("no media frame was decoded"),
            None => thread::sleep(Duration::from_millis(5)),
        }
    }
    panic!("media decoder did not return a first frame within 12 seconds");
}

#[test]
fn windows_mp3_preview_emits_pcm() {
    let (_dir, path) = generated_media(
        "mp3",
        &[
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000",
            "-t",
            "1",
            "-c:a",
            "libmp3lame",
        ],
    );
    let session = Session::start(test_source(path, true), 0).expect("MP3 session");
    let (header, frame) = first_frame(&session);
    assert!(header.audio);
    assert_eq!(header.width, 0);
    assert_eq!(header.height, 0);
    assert_eq!(
        frame.samples.len(),
        crate::media::audio_bytes(header, frame.tick)
    );
    assert!(
        frame.samples.iter().any(|byte| *byte != 0),
        "no audible samples"
    );
}

#[test]
fn windows_video_preview_keeps_audio() {
    let (_dir, path) = generated_media(
        "mp4",
        &[
            "-f",
            "lavfi",
            "-i",
            "color=c=red:size=128x72:rate=30",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=880:sample_rate=48000",
            "-t",
            "1",
            "-c:v",
            "mpeg4",
            "-c:a",
            "aac",
            "-pix_fmt",
            "yuv420p",
            "-shortest",
        ],
    );
    let session = Session::start(test_source(path, false), 0).expect("video session");
    let (header, frame) = first_frame(&session);
    assert!(header.audio, "video audio track was lost");
    assert_eq!(header.width, 96);
    assert_eq!(header.height, 54);
    assert_eq!(frame.pixels.len(), header.video_bytes());
    assert_eq!(
        frame.samples.len(),
        crate::media::audio_bytes(header, frame.tick)
    );
    assert!(
        frame.samples.iter().any(|byte| *byte != 0),
        "silent video preview"
    );
}

#[test]
fn windows_portrait_preview_keeps_all_frames_without_drift() {
    let (_dir, path) = generated_media(
        "mp4",
        &[
            "-f",
            "lavfi",
            "-i",
            "color=c=green:size=72x128:rate=30",
            "-t",
            "1",
            "-c:v",
            "mpeg4",
            "-pix_fmt",
            "yuv420p",
        ],
    );
    let session = Session::start(test_source(path, false), 0).expect("portrait session");
    let deadline = Instant::now() + Duration::from_secs(12);
    let mut frames = 0;
    let mut prepared = None;
    loop {
        assert!(Instant::now() < deadline, "portrait decoder deadline");
        match session.receive() {
            Some(Event::Prepared(header)) => prepared = Some(header),
            Some(Event::Packet(Packet::Frame(frame))) => {
                frames += 1;
                assert_eq!(
                    frame.pixels.len(),
                    prepared.as_ref().expect("header").video_bytes()
                );
            }
            Some(Event::Packet(Packet::End(_))) => break,
            Some(Event::Failed(error)) => panic!("portrait decoder failure: {error}"),
            None => thread::sleep(Duration::from_millis(5)),
        }
    }
    let header = prepared.expect("portrait header");
    assert_eq!((header.width, header.height), (36, 64));
    assert!(
        frames >= 20,
        "expected a full portrait stream, got {frames}"
    );
}

#[test]
fn video_output_fits_the_source_aspect_ratio_without_padding() {
    assert_eq!(fitted_video_size(520, 800, 1920.0, 1080.0), (520, 292));
    assert_eq!(fitted_video_size(520, 800, 1080.0, 1920.0), (450, 800));
    assert_eq!(fitted_video_size(128, 128, 1920.0, 1080.0), (128, 72));
}
