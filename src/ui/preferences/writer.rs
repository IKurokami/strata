// SPDX-License-Identifier: MIT

use std::{
    fs, io,
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};

use futures_channel::oneshot;

use super::{FileStamp, Preferences, file_stamp};

const SAVE_DELAY: Duration = Duration::from_millis(200);

#[derive(Clone)]
pub(super) struct Completion {
    pub revision: u64,
    pub path: PathBuf,
    pub result: Result<Option<FileStamp>, Arc<io::Error>>,
}

struct Save {
    revision: u64,
    path: PathBuf,
    preferences: Preferences,
    done: oneshot::Sender<Completion>,
}

enum Command {
    Wake,
    Flush(mpsc::SyncSender<Option<Completion>>),
}

pub(super) struct Writer {
    sender: mpsc::SyncSender<Command>,
    pending: Arc<Mutex<Option<Box<Save>>>>,
}

impl Writer {
    pub fn new() -> io::Result<Self> {
        let (sender, receiver) = mpsc::sync_channel(1);
        let pending = Arc::new(Mutex::new(None));
        let worker_pending = pending.clone();
        thread::Builder::new()
            .name("strata-preferences".into())
            .spawn(move || run(receiver, worker_pending))?;
        Ok(Self { sender, pending })
    }

    pub fn save(
        &self,
        revision: u64,
        path: PathBuf,
        preferences: Preferences,
    ) -> io::Result<oneshot::Receiver<Completion>> {
        let (done, receiver) = oneshot::channel();
        self.pending
            .lock()
            .map_err(|_| io::Error::from(io::ErrorKind::ConnectionAborted))?
            .replace(Box::new(Save {
                revision,
                path,
                preferences,
                done,
            }));
        match self.sender.try_send(Command::Wake) {
            Ok(()) | Err(mpsc::TrySendError::Full(_)) => {}
            Err(mpsc::TrySendError::Disconnected(_)) => {
                return Err(io::ErrorKind::ConnectionAborted.into());
            }
        }
        Ok(receiver)
    }

    pub fn flush(&self) -> io::Result<Option<Completion>> {
        let (sender, receiver) = mpsc::sync_channel(1);
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut command = Command::Flush(sender);
        loop {
            match self.sender.try_send(command) {
                Ok(()) => break,
                Err(mpsc::TrySendError::Full(returned)) if Instant::now() < deadline => {
                    command = returned;
                    thread::sleep(Duration::from_millis(2));
                }
                Err(mpsc::TrySendError::Full(_)) => {
                    return Err(io::ErrorKind::TimedOut.into());
                }
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    return Err(io::ErrorKind::ConnectionAborted.into());
                }
            }
        }
        receiver
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .map_err(|error| match error {
                mpsc::RecvTimeoutError::Timeout => io::ErrorKind::TimedOut.into(),
                mpsc::RecvTimeoutError::Disconnected => io::ErrorKind::ConnectionAborted.into(),
            })
    }
}

fn run(receiver: mpsc::Receiver<Command>, pending: Arc<Mutex<Option<Box<Save>>>>) {
    let mut last = None;
    while let Ok(command) = receiver.recv() {
        let mut save = match command {
            Command::Flush(reply) => {
                let _ = reply.send(last.clone());
                continue;
            }
            Command::Wake => {
                let Ok(mut pending) = pending.lock() else {
                    return;
                };
                let Some(save) = pending.take() else { continue };
                save
            }
        };
        let deadline = Instant::now() + SAVE_DELAY;
        let mut flush = None;
        loop {
            match receiver.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(Command::Wake) => {
                    let Ok(mut pending) = pending.lock() else {
                        return;
                    };
                    if let Some(next) = pending.take() {
                        save = next;
                    }
                }
                Ok(Command::Flush(reply)) => {
                    flush = Some(reply);
                    break;
                }
                Err(_) => break,
            }
            if Instant::now() >= deadline {
                break;
            }
        }
        let result = (|| {
            let contents = toml::to_string_pretty(&save.preferences).map_err(io::Error::other)?;
            if let Some(parent) = save.path.parent() {
                fs::create_dir_all(parent)?;
            }
            crate::storage::atomic_write_config(&save.path, contents.as_bytes())?;
            Ok(file_stamp(&save.path))
        })();
        let completion = Completion {
            revision: save.revision,
            path: save.path,
            result: result.map_err(Arc::new),
        };
        let _ = save.done.send(completion.clone());
        last = Some(completion);
        if let Some(reply) = flush {
            let _ = reply.send(last.clone());
        }
    }
}

#[cfg(test)]
mod tests;
