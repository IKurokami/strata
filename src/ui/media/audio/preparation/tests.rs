// SPDX-License-Identifier: MIT

use super::*;
use crate::{media::Header, ui::media::DecodedMedia};
use gtk::{glib, prelude::*, subclass::prelude::*};
use std::{rc::Rc, time::Instant};

fn header() -> Header {
    Header {
        width: 128,
        height: 72,
        audio: true,
        duration_us: 2_000_000,
        start_tick: 0,
    }
}

fn pump_until(mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !done() {
        assert!(
            Instant::now() < deadline,
            "audio preparation did not settle"
        );
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn slow_audio_setup_keeps_gtk_responsive_and_can_be_cancelled() {
    crate::test_support::gtk_test(
        "ui::media::audio::preparation::tests::slow_audio_setup_keeps_gtk_responsive_and_can_be_cancelled",
        || {
            let (release, blocked) = mpsc::channel();
            let (entered, ready) = mpsc::channel();
            let preparation = AudioPreparation::start_with(move || {
                entered.send(()).expect("setup entered");
                blocked.recv().expect("release setup");
                Err("cancelled test setup".into())
            })
            .expect("worker spawn")
            .expect("setup slot");
            ready
                .recv_timeout(Duration::from_secs(5))
                .expect("worker started");
            // Other previews retry instead of launching an unbounded set of device probes.
            assert!(
                AudioPreparation::start(true, 0.0, Duration::from_millis(100))
                    .expect("second setup")
                    .is_none()
            );
            let media: DecodedMedia = glib::Object::new();
            media.imp().audio_header.set(Some(header()));
            media.imp().audio_preparation.replace(Some(preparation));
            let heartbeat = Rc::new(Cell::new(false));
            let delivered = heartbeat.clone();
            glib::idle_add_local_once(move || delivered.set(true));
            media.tick().expect("pending tick");
            pump_until(|| heartbeat.get());
            assert!(!media.is_prepared());
            media.close();
            assert!(media.imp().audio_preparation.borrow().is_none());
            release
                .send(())
                .expect("cancelled setup returns without a UI join");
            pump_until(|| !INITIALIZING.load(Ordering::Acquire));
            assert!(
                !media.is_prepared(),
                "late result resurrected a closed preview"
            );
        },
    );
}

#[test]
fn prepared_audio_uses_current_controls_and_restart_discards_pending_setup() {
    crate::test_support::gtk_test(
        "ui::media::audio::preparation::tests::prepared_audio_uses_current_controls_and_restart_discards_pending_setup",
        || {
            let (release, blocked) = mpsc::channel();
            let preparation = AudioPreparation::start_with(move || {
                blocked.recv().expect("release setup");
                PreparedOutput::prepare(true, 0.0, Duration::from_millis(100))
            })
            .expect("worker spawn")
            .expect("setup slot");
            let media: DecodedMedia = glib::Object::new();
            media.imp().audio_header.set(Some(header()));
            media.imp().audio_preparation.replace(Some(preparation));
            media.set_muted(false);
            media.set_volume(0.35);
            media.imp().fade.set(Some(0.5));
            release.send(()).expect("finish preparation");
            pump_until(|| {
                media.tick().expect("prepare tick");
                media.is_prepared()
            });
            assert!(media.has_audio());
            let volume = media
                .imp()
                .audio
                .borrow()
                .as_ref()
                .expect("output")
                .effective_volume();
            assert!(
                (volume - 0.175).abs() < 1e-6,
                "latest volume and fade were not applied: {volume}"
            );
            pump_until(|| !INITIALIZING.load(Ordering::Acquire));
            let (release, blocked) = mpsc::channel();
            let (entered, ready) = mpsc::channel();
            let pending = AudioPreparation::start_with(move || {
                entered.send(()).expect("obsolete setup entered");
                blocked.recv().expect("release obsolete setup");
                Err("obsolete test setup".into())
            })
            .expect("worker spawn")
            .expect("setup slot");
            ready
                .recv_timeout(Duration::from_secs(5))
                .expect("obsolete worker started");
            media.imp().audio_header.set(Some(header()));
            media.imp().audio_preparation.replace(Some(pending));
            media.restart_at(1_000_000);
            assert!(media.imp().audio_header.get().is_none());
            assert!(media.imp().audio_preparation.borrow().is_none());
            media.close();
            release.send(()).expect("retire obsolete setup");
            pump_until(|| !INITIALIZING.load(Ordering::Acquire));
        },
    );
}

#[test]
fn abandoned_ready_audio_releases_its_pipeline() {
    gst::init().expect("GStreamer");
    let (sender, receiver) = mpsc::channel();
    let preparation = AudioPreparation::start_with(move || {
        let prepared = PreparedOutput::prepare(true, 0.0, Duration::from_millis(100))?;
        sender
            .send(prepared.output.as_ref().expect("pipeline").0.clone())
            .expect("observe pipeline");
        Ok(prepared)
    })
    .expect("worker spawn")
    .expect("setup slot");
    let pipeline = receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("pipeline created");
    let deadline = Instant::now() + Duration::from_secs(5);
    while INITIALIZING.load(Ordering::Acquire) {
        assert!(Instant::now() < deadline, "audio worker did not return");
        std::thread::sleep(Duration::from_millis(1));
    }
    drop(preparation);
    let deadline = Instant::now() + Duration::from_secs(5);
    while pipeline.current_state() != gst::State::Null {
        assert!(
            Instant::now() < deadline,
            "abandoned result retained audio device"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}
