// SPDX-License-Identifier: MIT

//! Windows provider transport.
//!
//! Providers are invoked without a shell and each request runs in a bounded
//! Win32 Job Object. A short-lived process per request avoids keeping an
//! untrusted provider alive between selections while still supporting the
//! same newline-delimited protocol as the Unix transport.

use super::protocol::{FRAME_LIMIT, checked_reply};
use super::{Manifest, PATH_LIMIT, Registration, Reply, Request};
use std::{
    collections::{BTreeMap, VecDeque},
    io,
    path::Path,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
    },
    thread,
    time::{Duration, Instant},
};

const DEADLINE: Duration = Duration::from_secs(8);
const UPDATE_LIMIT: usize = 32;

pub(crate) enum Update {
    Reply(Box<Reply>),
    Offline {
        unsent: Vec<u64>,
        in_flight: Option<u64>,
    },
}

pub(crate) struct Requests {
    refresh: SyncSender<Request>,
    activate: SyncSender<Request>,
    closed: Arc<AtomicBool>,
}

impl Drop for Requests {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Relaxed);
    }
}

impl Requests {
    pub(crate) fn try_send(&self, request: Request) -> Result<(), TrySendError<Request>> {
        if !request.valid() || !super::selection_fits(&request.paths) {
            return Err(TrySendError::Full(request));
        }
        if request.method == "activate" {
            self.activate.try_send(request)
        } else {
            self.refresh.try_send(request)
        }
    }
}

#[derive(Default)]
struct UpdateState {
    replies: VecDeque<Update>,
    reply_count: usize,
    global: Option<Reply>,
    scopes: BTreeMap<String, Option<u64>>,
    closed: bool,
}

impl UpdateState {
    fn flush_events(&mut self) {
        self.replies.extend(
            self.global
                .take()
                .map(|reply| Update::Reply(Box::new(reply))),
        );
        let mut grouped: BTreeMap<Option<u64>, Vec<String>> = BTreeMap::new();
        for (path, revision) in std::mem::take(&mut self.scopes) {
            grouped.entry(revision).or_default().push(path);
        }
        self.replies
            .extend(grouped.into_iter().map(|(revision, paths)| {
                Update::Reply(Box::new(Reply {
                    version: 1,
                    event: Some("invalidate".into()),
                    paths: Some(paths),
                    revision,
                    ..Reply::default()
                }))
            }));
    }
}

#[derive(Default)]
pub(crate) struct Updates {
    state: Mutex<UpdateState>,
    changed: Condvar,
}

impl Updates {
    pub(crate) fn drain(&self) -> (Vec<Update>, bool) {
        let mut state = self.state.lock().expect("provider update queue");
        state.flush_events();
        state.reply_count = 0;
        (state.replies.drain(..).collect(), state.closed)
    }

    fn has_room(&self) -> bool {
        self.state
            .lock()
            .expect("provider update queue")
            .reply_count
            < UPDATE_LIMIT
    }

    fn publish(&self, reply: Reply) {
        let mut state = self.state.lock().expect("provider update queue");
        if reply.event.is_some() {
            if let Some(paths) = &reply.paths {
                for path in paths {
                    let revision = state.scopes.entry(path.clone()).or_default();
                    *revision = (*revision).into_iter().chain(reply.revision).max();
                }
                if state.scopes.len() > PATH_LIMIT
                    || state.scopes.keys().map(String::len).sum::<usize>() > 65536
                {
                    let revision = state
                        .scopes
                        .values()
                        .filter_map(|r| *r)
                        .chain(state.global.as_ref().and_then(|r| r.revision))
                        .max();
                    state.scopes.clear();
                    state.global = Some(Reply {
                        paths: None,
                        revision,
                        ..reply
                    });
                }
            } else {
                let revision = state
                    .global
                    .as_ref()
                    .and_then(|r| r.revision)
                    .into_iter()
                    .chain(reply.revision)
                    .max();
                state.scopes.retain(|_, at| {
                    at.is_some_and(|at| revision.is_none_or(|revision| at > revision))
                });
                state.global = Some(Reply { revision, ..reply });
            }
        } else {
            assert!(state.reply_count < UPDATE_LIMIT);
            state.flush_events();
            state.replies.push_back(Update::Reply(Box::new(reply)));
            state.reply_count += 1;
        }
        self.changed.notify_one();
    }

    fn offline(&self, unsent: Vec<u64>, in_flight: Option<u64>) {
        let mut state = self.state.lock().expect("provider update queue");
        state.flush_events();
        state
            .replies
            .push_back(Update::Offline { unsent, in_flight });
        state.reply_count += 1;
        self.changed.notify_one();
    }

    fn close(&self) {
        self.state.lock().expect("provider update queue").closed = true;
        self.changed.notify_all();
    }
}

pub(crate) struct Client {
    pub requests: Requests,
    pub updates: Arc<Updates>,
}

struct Incoming {
    refresh: Receiver<Request>,
    activate: Receiver<Request>,
    closed: Arc<AtomicBool>,
}

impl Incoming {
    fn next(&self) -> Result<Option<Request>, ()> {
        match self.activate.try_recv() {
            Ok(request) => return Ok(Some(request)),
            Err(TryRecvError::Disconnected) => {}
            Err(TryRecvError::Empty) => {}
        }
        match self.refresh.try_recv() {
            Ok(request) => Ok(Some(request)),
            Err(TryRecvError::Disconnected) => Err(()),
            Err(TryRecvError::Empty) => Ok(None),
        }
    }

    fn discard(&self) -> Vec<u64> {
        self.activate
            .try_iter()
            .chain(self.refresh.try_iter())
            .map(|request| request.id)
            .collect()
    }
}

pub(crate) fn start(registration: Registration) -> Client {
    let (refresh, refresh_rx) = mpsc::sync_channel(16);
    let (activate, activate_rx) = mpsc::sync_channel(8);
    let updates = Arc::new(Updates::default());
    let out = updates.clone();
    let closed = Arc::new(AtomicBool::new(false));
    let worker_closed = closed.clone();
    thread::spawn(move || {
        let incoming = Incoming {
            refresh: refresh_rx,
            activate: activate_rx,
            closed: worker_closed,
        };
        let mut last_diagnostic = None;
        loop {
            let first = loop {
                if incoming.closed.load(Ordering::Relaxed) {
                    out.close();
                    return;
                }
                if out.has_room() {
                    match incoming.next() {
                        Ok(Some(request)) => break request,
                        Err(()) => {
                            out.close();
                            return;
                        }
                        Ok(None) => {}
                    }
                }
                thread::sleep(Duration::from_millis(25));
            };
            match run_request(&registration.manifest, &out, first) {
                Ok(()) => {}
                Err(failure) => {
                    if last_diagnostic
                        .is_none_or(|at: Instant| at.elapsed() >= Duration::from_secs(10))
                    {
                        eprintln!(
                            "Strata provider {} unavailable: {}",
                            registration.manifest.id, failure.reason
                        );
                        last_diagnostic = Some(Instant::now());
                    }
                    let mut unsent = incoming.discard();
                    if let Some(id) = failure.unsent {
                        unsent.push(id);
                    }
                    out.offline(unsent, failure.in_flight);
                    thread::sleep(Duration::from_secs(2));
                }
            }
        }
    });
    Client {
        requests: Requests {
            refresh,
            activate,
            closed,
        },
        updates,
    }
}

struct Failure {
    reason: &'static str,
    unsent: Option<u64>,
    in_flight: Option<u64>,
}

fn run_request(manifest: &Manifest, updates: &Updates, request: Request) -> Result<(), Failure> {
    if !request.valid() {
        return Err(Failure {
            reason: "invalid host request",
            unsent: Some(request.id),
            in_flight: None,
        });
    }
    let mut frame = serde_json::to_vec(&request).map_err(|_| Failure {
        reason: "request serialization failure",
        unsent: Some(request.id),
        in_flight: None,
    })?;
    frame.push(b'\n');
    if frame.len() > FRAME_LIMIT {
        return Err(Failure {
            reason: "request frame limit",
            unsent: Some(request.id),
            in_flight: None,
        });
    }

    let executable = Path::new(&manifest.command[0]);
    let args: Vec<_> = manifest.command[1..]
        .iter()
        .map(|argument| std::ffi::OsString::from(argument.as_str()))
        .collect();
    let arg_refs: Vec<_> = args.iter().map(|arg| arg.as_os_str()).collect();
    let limits = strata_win_shell::JobLimits {
        memory_limit_bytes: Some(1024 * 1024 * 1024),
        active_process_limit: Some(16),
        kill_on_job_close: true,
        restrict_ui: true,
    };
    let output =
        strata_win_shell::run_in_job(executable, &arg_refs, Some(&frame), &limits, DEADLINE, None)
            .map_err(|_| Failure {
                reason: "provider process failure",
                unsent: None,
                in_flight: Some(request.id),
            })?;
    if output.exit_code != 0 {
        return Err(Failure {
            reason: "provider exited unsuccessfully",
            unsent: None,
            in_flight: Some(request.id),
        });
    }

    let mut buffer = output.stdout;
    let mut matched = false;
    for frame in take_frames(&mut buffer).map_err(|_| Failure {
        reason: "response frame limit",
        unsent: None,
        in_flight: Some(request.id),
    })? {
        let reply = checked_reply(&frame, manifest).map_err(|_| Failure {
            reason: "invalid provider response",
            unsent: None,
            in_flight: Some(request.id),
        })?;
        if reply.event.is_some() {
            updates.publish(reply);
        } else if reply.id == Some(request.id) {
            if !reply.matches(&request) {
                return Err(Failure {
                    reason: "invalid method response",
                    unsent: None,
                    in_flight: Some(request.id),
                });
            }
            updates.publish(reply);
            matched = true;
        }
    }
    if matched {
        Ok(())
    } else {
        Err(Failure {
            reason: "provider returned no matching response",
            unsent: None,
            in_flight: Some(request.id),
        })
    }
}

fn take_frames(buffer: &mut Vec<u8>) -> io::Result<Vec<Vec<u8>>> {
    let mut frames = Vec::new();
    let mut start = 0;
    for (end, byte) in buffer.iter().enumerate() {
        if *byte == b'\n' {
            if end - start > FRAME_LIMIT {
                return Err(io::Error::other("frame limit"));
            }
            frames.push(buffer[start..end].to_vec());
            start = end + 1;
        }
    }
    buffer.drain(..start);
    if buffer.len() > FRAME_LIMIT {
        return Err(io::Error::other("frame limit"));
    }
    Ok(frames)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_parser_keeps_partial_tail_bounded() {
        let mut buffer = b"{\"version\":1}\npartial".to_vec();
        let frames = take_frames(&mut buffer).expect("frames");
        assert_eq!(frames, vec![b"{\"version\":1}".to_vec()]);
        assert_eq!(buffer, b"partial");
    }

    #[test]
    fn frame_parser_rejects_an_unbounded_line() {
        let mut buffer = vec![b'x'; FRAME_LIMIT + 1];
        assert!(take_frames(&mut buffer).is_err());
    }
}
