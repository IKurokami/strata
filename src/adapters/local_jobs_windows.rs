// SPDX-License-Identifier: MIT

//! Win32 custom-action runner boundary.
//!
//! Linux action scripts rely on POSIX process groups for cancellation, Unix
//! owner-only temporary directories, nonblocking pipe FDs and signal-based
//! reaping. A naive Windows `Command::spawn` port would lose descendants on
//! cancel and keep untrusted helper programs running after their job ends.
//! Do not launch an uncontained subprocess or run shell scripts on the GTK
//! thread. A future Windows runner will need owned Win32 Job Objects, an
//! explicitly trusted executable and cancellation of the whole process tree.
use std::rc::Rc;

use crate::services::{
    ActionEventSink, ActionRunEvent, ActionRunRequest, ActionRunner, CancelHandle,
};

#[derive(Default)]
pub(crate) struct LocalActionRunner;

impl LocalActionRunner {
    /// Instantiation is intentionally free of filesystem, PATH and process
    /// probes, making it safe during first GTK window construction.
    pub(crate) fn new() -> Rc<Self> {
        Rc::new(Self)
    }
}

impl ActionRunner for LocalActionRunner {
    fn run(&self, request: &ActionRunRequest, sink: ActionEventSink) -> CancelHandle {
        let detail = request
            .action
            .unavailable_reason()
            .unwrap_or(
                "Custom actions on Windows require native process-tree isolation before they can run",
            );
        // Completion is explicit. Do not report Exited(0) or leave a hanging
        // cancellable job when execution is unavailable.
        sink(ActionRunEvent::Failed(detail.to_owned()));
        Rc::new(|| {})
    }
}
