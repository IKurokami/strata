// SPDX-License-Identifier: MIT

//! Real worker isolation test. Simulate a kernel call stuck on a mapped drive
//! without touching any foreign drive letter on the host.
use super::*;
use std::{
    sync::{
        Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

struct PausedDrive {
    path: PathBuf,
    entered: AtomicBool,
    released: Mutex<bool>,
    ready: Condvar,
}

impl PausedDrive {
    fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.ready.notify_all();
    }
}

static PAUSED: Mutex<Option<Arc<PausedDrive>>> = Mutex::new(None);
static NEXT: AtomicU64 = AtomicU64::new(0);

/// Test-only hook executed on the worker thread before any OS file operation.
pub(super) fn pause_if_simulated_offline(operation: &LocalOperation) {
    let LocalOperation::CreateDirectory(path) = operation else {
        return;
    };
    let pause = PAUSED.lock().unwrap().as_ref().cloned();
    let Some(pause) = pause.filter(|gate| gate.path.as_path() == path) else {
        return;
    };
    pause.entered.store(true, Ordering::Release);
    let mut released = pause.released.lock().unwrap();
    while !*released {
        released = pause.ready.wait(released).unwrap();
    }
}

struct ResetPause(Arc<PausedDrive>);

impl Drop for ResetPause {
    fn drop(&mut self) {
        self.0.release();
        *PAUSED.lock().unwrap() = None;
    }
}

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "strata-drive-isolation-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("create temporary root");
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).expect("clean temporary root");
    }
}

fn completion(service: &LocalOperationService, id: u64) -> io::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match service.try_recv() {
            Ok(result) if result.id == id => return result.result,
            Ok(other) => panic!("unexpected completion {}", other.id),
            Err(TryRecvError::Empty) => {
                assert!(Instant::now() < deadline, "operation did not complete");
                thread::sleep(Duration::from_millis(2));
            }
            Err(TryRecvError::Disconnected) => panic!("completion channel disconnected"),
        }
    }
}

#[test]
fn simulated_offline_mapped_drive_cannot_stall_another_drive_worker() {
    let scratch = Scratch::new();
    let local_letter = drive_letter(&scratch.0).expect("temporary folder on drive letter");
    let mapped_letter = (b'A'..=b'Z')
        .find(|&letter| letter != local_letter && letter != system_drive_letter())
        .expect("available test drive letter");
    let fake_offline = PathBuf::from(format!(
        "{}:\\strata-offline-test-{}\\new",
        mapped_letter as char,
        std::process::id()
    ));
    // No operation ever reaches the OS at this fake path: the worker pauses
    // first, and cancellation is accepted before the irreversible step.
    let pause = Arc::new(PausedDrive {
        path: fake_offline.clone(),
        entered: AtomicBool::new(false),
        released: Mutex::new(false),
        ready: Condvar::new(),
    });
    *PAUSED.lock().unwrap() = Some(Arc::clone(&pause));
    let _guard = ResetPause(Arc::clone(&pause));

    let service = LocalOperationService::new().expect("create independent workers");
    let stalled = service
        .submit(LocalOperation::CreateDirectory(fake_offline))
        .expect("schedule mapped drive work");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !pause.entered.load(Ordering::Acquire) {
        assert!(Instant::now() < deadline, "mapped worker did not start");
        thread::sleep(Duration::from_millis(2));
    }

    let healthy_path = scratch.0.join("healthy-local-folder");
    let healthy = service
        .submit(LocalOperation::CreateDirectory(healthy_path.clone()))
        .expect("submit an unaffected local drive operation");
    completion(&service, healthy).expect("local mkdir succeeds while another drive is blocked");
    assert!(healthy_path.is_dir());

    assert!(
        service.cancel(stalled),
        "cancel stalled work before any filesystem call"
    );
    pause.release();
    assert_eq!(
        completion(&service, stalled)
            .expect_err("cancelled operation")
            .kind(),
        io::ErrorKind::Interrupted
    );
}
