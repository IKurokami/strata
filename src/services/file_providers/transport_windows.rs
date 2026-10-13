// SPDX-License-Identifier: MIT

//! Win32 file-provider transport is unavailable until named-pipe identity
//! verification and Job Object process-tree cancellation are implemented.
//! No child process or socket may start from a file selection or at launch.

use std::sync::{Arc, mpsc::TrySendError};

use super::{Registration, Reply, Request};

pub(crate) enum Update {
    Reply(Box<Reply>),
    Offline {
        unsent: Vec<u64>,
        in_flight: Option<u64>,
    },
}

pub(crate) struct Requests;

impl Requests {
    pub(crate) fn try_send(&self, request: Request) -> Result<(), TrySendError<Request>> {
        Err(TrySendError::Disconnected(request))
    }
}

#[derive(Default)]
pub(crate) struct Updates;

impl Updates {
    pub(crate) fn drain(&self) -> (Vec<Update>, bool) {
        (Vec::new(), true)
    }
}

pub(crate) struct Client {
    pub requests: Requests,
    pub updates: Arc<Updates>,
}

/// Exists only to preserve the normal client API for GUI callers. Discovery
/// never returns registrations on Windows, so no provider is constructed.
pub(crate) fn start(_registration: Registration) -> Client {
    Client {
        requests: Requests,
        updates: Arc::new(Updates),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_provider_requests_without_executing_helpers() {
        let client = start(Registration {
            manifest: super::super::Manifest {
                version: 1,
                id: "custom".to_owned(),
                name: None,
                icon: None,
                command: vec!["cmd.exe".to_owned()],
                icons: Default::default(),
            },
            icons: Default::default(),
        });
        let request = Request {
            version: 1,
            id: 42,
            method: "activate".to_owned(),
            paths: vec!["C:\\test.txt".to_owned()],
            background: false,
            action: Some("run".to_owned()),
            context: None,
        };
        assert!(matches!(
            client.requests.try_send(request),
            Err(TrySendError::Disconnected(_))
        ));
        let (updates, closed) = client.updates.drain();
        assert!(closed && updates.is_empty());
    }
}
