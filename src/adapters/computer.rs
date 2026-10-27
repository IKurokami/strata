// SPDX-License-Identifier: MIT

use crate::{
    model::{EntryKind, FileEntry, Location, MetadataValue},
    services::{DirectoryEvent, DirectoryRequest, LoadHandle},
};
use gtk::{gio, glib};
use std::rc::Rc;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock, mpsc},
    time::{Duration, Instant},
};
use strata_win_shell::volume::{VolumeInfo, VolumeSpace};

#[derive(Clone)]
pub(crate) struct DriveDetails {
    pub volume: VolumeInfo,
    pub space: Option<VolumeSpace>,
}

type Reply = futures_channel::oneshot::Sender<DriveDetails>;
#[derive(Default)]
struct Queries {
    cached: HashMap<PathBuf, (Instant, DriveDetails)>,
    waiting: HashMap<PathBuf, Vec<Reply>>,
}

fn queries() -> &'static Mutex<Queries> {
    static QUERIES: OnceLock<Mutex<Queries>> = OnceLock::new();
    QUERIES.get_or_init(Mutex::default)
}

fn workers(remote: bool) -> &'static mpsc::Sender<VolumeInfo> {
    static LOCAL: OnceLock<mpsc::Sender<VolumeInfo>> = OnceLock::new();
    static REMOTE: OnceLock<mpsc::Sender<VolumeInfo>> = OnceLock::new();
    let pool = if remote { &REMOTE } else { &LOCAL };
    pool.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<VolumeInfo>();
        let rx = Arc::new(Mutex::new(rx));
        for index in 0..if remote { 1 } else { 3 } {
            let rx = rx.clone();
            std::thread::Builder::new()
                .name(format!(
                    "drive-info-{}-{index}",
                    if remote { "remote" } else { "local" }
                ))
                .spawn(move || {
                    loop {
                        let request = rx.lock().unwrap_or_else(|e| e.into_inner()).recv();
                        let Ok(mut volume) = request else { break };
                        let started = Instant::now();
                        strata_win_shell::volume::populate_volume_info(&mut volume);
                        let space = strata_win_shell::volume::volume_space(&volume.root).ok();
                        let details = DriveDetails { volume, space };
                        let mut state = queries().lock().unwrap_or_else(|e| e.into_inner());
                        let root = details.volume.root.clone();
                        let replies = state.waiting.remove(&root).unwrap_or_default();
                        state.cached.insert(root, (Instant::now(), details.clone()));
                        drop(state);
                        tracing::debug!(
                            elapsed_micros = started.elapsed().as_micros(),
                            "drive metadata ready"
                        );
                        for reply in replies {
                            let _ = reply.send(details.clone());
                        }
                    }
                })
                .expect("drive information worker");
        }
        tx
    })
}

pub(crate) async fn details(volume: VolumeInfo) -> Option<DriveDetails> {
    let root = volume.root.clone();
    let (tx, rx) = futures_channel::oneshot::channel();
    let mut state = queries().lock().unwrap_or_else(|e| e.into_inner());
    if let Some((when, cached)) = state.cached.get(&root)
        && when.elapsed() < Duration::from_secs(15)
    {
        return Some(cached.clone());
    }
    let first = !state.waiting.contains_key(&root);
    state.waiting.entry(root.clone()).or_default().push(tx);
    drop(state);
    let remote = volume.drive_type == strata_win_shell::volume::DriveType::Remote;
    if first && workers(remote).send(volume).is_err() {
        queries()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .waiting
            .remove(&root);
    }
    rx.await.ok()
}

pub(crate) fn is_root(location: &Location) -> bool {
    location.uri_value() == Some("computer:///")
}

pub(crate) fn enumerate(request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
    let task = glib::MainContext::default().spawn_local(async move {
        let Ok(volumes) = gio::spawn_blocking(strata_win_shell::volume::drive_roots).await else {
            emit(DirectoryEvent::Failed {
                request_id: request.id,
                message: crate::i18n::tr("Unable to read drives"),
            });
            return;
        };
        emit(DirectoryEvent::Batch {
            request_id: request.id,
            entries: volumes
                .into_iter()
                .map(|volume| {
                    let name = volume.root.as_os_str().to_owned();
                    FileEntry {
                        location: Location::local(&volume.root),
                        native_name: name.clone(),
                        display_name: name.to_string_lossy().into_owned(),
                        thumbnail_path: None,
                        kind: EntryKind::Directory,
                        size: MetadataValue::Unavailable,
                        modified_unix_seconds: MetadataValue::Unavailable,
                        recent_unix_seconds: MetadataValue::Unavailable,
                        recent_uri: None,
                        mode: MetadataValue::Unavailable,
                        image_dimensions: MetadataValue::Unavailable,
                        child_count: MetadataValue::Unavailable,
                        duration_seconds: MetadataValue::Unavailable,
                        is_hidden: false,
                    }
                })
                .collect(),
        });
        emit(DirectoryEvent::Finished {
            request_id: request.id,
            truncated: false,
            can_trash: Some(false),
            can_delete: Some(false),
        });
    });
    LoadHandle::new(move || task.abort())
}
