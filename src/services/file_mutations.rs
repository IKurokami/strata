// SPDX-License-Identifier: MIT

use super::listeners::{ListenerGuard, Listeners};
use crate::model::Location;
use std::rc::Rc;

#[cfg(any(windows, test))]
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, LazyLock, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

type Callback = Rc<dyn Fn(&[Location])>;
pub(crate) type MutationWatch = ListenerGuard<Callback>;

thread_local! {
    static WATCHERS: Rc<Listeners<Callback>> = Rc::new(Listeners::new());
}

pub(crate) fn watch(callback: impl Fn(&[Location]) + 'static) -> MutationWatch {
    WATCHERS.with(|watchers| watchers.add(Rc::new(callback)))
}

pub(crate) fn prepare(locations: &[Location]) {
    WATCHERS.with(|watchers| watchers.notify(|callback| callback(locations)));
}

pub(crate) async fn release(locations: &[Location]) {
    prepare(locations);
    wait_for_workers(locations).await;
}

pub(crate) fn contains(roots: &[Location], location: &Location) -> bool {
    let mut current = Some(location.clone());
    while let Some(location) = current {
        if roots.contains(&location) {
            return true;
        }
        current = location.parent();
    }
    false
}

#[cfg(any(windows, test))]
struct WorkerState {
    path: PathBuf,
    cancelled: Arc<AtomicBool>,
}

#[cfg(any(windows, test))]
static WORKERS: LazyLock<Mutex<Vec<Weak<WorkerState>>>> = LazyLock::new(Mutex::default);

/// The worker, rather than its UI handle, owns this lease until all child files close.
#[cfg(any(windows, test))]
pub(crate) struct WorkerLease {
    _state: Arc<WorkerState>,
}

#[cfg(any(windows, test))]
pub(crate) fn worker(path: &Path, cancelled: Arc<AtomicBool>) -> WorkerLease {
    let state = Arc::new(WorkerState {
        path: path.to_path_buf(),
        cancelled,
    });
    let mut workers = WORKERS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    workers.retain(|worker| worker.strong_count() > 0);
    workers.push(Arc::downgrade(&state));
    WorkerLease { _state: state }
}

/// Poll teardown without joining a decoder or waiting on its pipes on GTK's thread.
pub(crate) async fn wait_for_workers(_locations: &[Location]) {
    #[cfg(any(windows, test))]
    let started = Instant::now();
    #[cfg(any(windows, test))]
    loop {
        let active = {
            let mut workers = WORKERS
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let mut active = false;
            workers.retain(|worker| {
                let Some(worker) = worker.upgrade() else {
                    return false;
                };
                if contains(_locations, &Location::local(&worker.path)) {
                    worker.cancelled.store(true, Ordering::Release);
                    active = true;
                }
                true
            });
            active
        };
        if !active {
            return;
        }
        if started.elapsed() >= Duration::from_secs(3) {
            tracing::warn!("preview worker teardown timed out before file operation");
            return;
        }
        gtk::glib::timeout_future(Duration::from_millis(5)).await;
    }
}

#[cfg(test)]
mod tests;
