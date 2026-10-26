// SPDX-License-Identifier: MIT

use super::*;
use std::{
    fs::OpenOptions,
    os::windows::fs::OpenOptionsExt,
    thread,
    time::{Duration, Instant},
};

#[test]
fn native_mutations_wait_for_preview_file_handles_without_blocking_the_main_context() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("main context");
    let context = glib::MainContext::default();
    for deleting in [false, true] {
        let root = tempfile::tempdir().expect("preview fixture");
        let source = root.path().join("preview.json");
        let destination = root.path().join("renamed.json");
        std::fs::write(&source, b"{\"preview\":true}").expect("source");
        let handle = OpenOptions::new()
            .read(true)
            .share_mode(3)
            .open(&source)
            .expect("preview handle without delete sharing");
        let cancelled = Arc::new(AtomicBool::new(false));
        let lease = crate::services::file_mutations::worker(&source, cancelled.clone());
        let worker = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(3);
            while !cancelled.load(AtomicOrdering::Acquire) && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(1));
            }
            assert!(cancelled.load(AtomicOrdering::Acquire));
            thread::sleep(Duration::from_millis(40));
            drop(handle);
            drop(lease);
        });
        let events = Rc::new(RefCell::new(Vec::new()));
        let captured = events.clone();
        let emit: Rc<dyn Fn(OperationEvent)> =
            Rc::new(move |event| captured.borrow_mut().push(event));
        let operation = if deleting {
            permanently_delete_locations(
                OperationRequestId(901),
                vec![Location::local(&source)],
                emit,
            )
        } else {
            async_rename_no_replace(
                source.clone(),
                destination.clone(),
                OperationRequestId(902),
                emit,
            )
        };
        let heartbeat = Rc::new(std::cell::Cell::new(false));
        let beat = heartbeat.clone();
        glib::timeout_add_local_once(Duration::from_millis(5), move || beat.set(true));
        let deadline = Instant::now() + Duration::from_secs(10);
        while events.borrow().is_empty() && Instant::now() < deadline {
            while context.pending() {
                context.iteration(false);
            }
            thread::sleep(Duration::from_millis(1));
        }
        worker.join().expect("preview teardown");
        assert!(heartbeat.get(), "preview teardown blocked the main loop");
        assert!(
            events.borrow().iter().any(|event| matches!(
                event,
                OperationEvent::Deleted { .. } | OperationEvent::Renamed { .. }
            )),
            "mutation failed: {:?}",
            events.borrow()
        );
        assert!(!source.exists());
        assert_eq!(destination.exists(), !deleting);
        drop(operation);
    }
}
