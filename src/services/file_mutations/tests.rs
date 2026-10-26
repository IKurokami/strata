// SPDX-License-Identifier: MIT

use super::*;
use std::{cell::Cell, thread};

#[test]
fn teardown_wait_cancels_only_affected_workers_and_keeps_the_main_context_running() {
    let root = tempfile::tempdir().expect("mutation fixture");
    let cancelled = Arc::new(AtomicBool::new(false));
    let unrelated_cancelled = Arc::new(AtomicBool::new(false));
    let lease = worker(&root.path().join("folder/video.mp4"), cancelled.clone());
    let _unrelated = worker(
        &root.path().join("folder-other/video.mp4"),
        unrelated_cancelled.clone(),
    );
    let stopped = cancelled.clone();
    let worker = thread::spawn(move || {
        let started = Instant::now();
        while !stopped.load(Ordering::Acquire) && started.elapsed() < Duration::from_secs(2) {
            thread::sleep(Duration::from_millis(1));
        }
        assert!(
            stopped.load(Ordering::Acquire),
            "affected worker was not cancelled"
        );
        thread::sleep(Duration::from_millis(40));
        drop(lease);
    });
    let context = gtk::glib::MainContext::new();
    let heartbeat = Rc::new(Cell::new(false));
    context
        .with_thread_default(|| {
            let beat = heartbeat.clone();
            context.spawn_local(async move {
                gtk::glib::timeout_future(Duration::from_millis(5)).await;
                beat.set(true);
            });
            context.block_on(wait_for_workers(&[Location::local(
                root.path().join("folder"),
            )]));
        })
        .expect("private context");
    worker.join().expect("decoder teardown");
    assert!(heartbeat.get(), "teardown blocked the event loop");
    assert!(!unrelated_cancelled.load(Ordering::Acquire));
}

#[test]
fn mutation_notifications_reach_all_live_consumers_and_unsubscribe_on_drop() {
    let count = Rc::new(Cell::new(0));
    let first = count.clone();
    let guard = watch(move |_| first.set(first.get() + 1));
    let second = count.clone();
    let _other = watch(move |_| second.set(second.get() + 10));
    prepare(&[Location::local("folder")]);
    assert_eq!(count.get(), 11);
    drop(guard);
    prepare(&[Location::local("folder")]);
    assert_eq!(count.get(), 21);
}
