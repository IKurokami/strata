// SPDX-License-Identifier: MIT

//! Win32 custom-action runner using Job Objects for process-tree containment.
//!
//! Executes custom action programs and scripts inside an isolated Win32 Job Object
//! configured with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`. When cancelled or finished,
//! Windows terminates all child processes in the tree, preventing runaway processes.

use std::{
    path::Path,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use crate::services::{
    ActionEventSink, ActionProgram, ActionRunEvent, ActionRunRequest, ActionRunner, CancelHandle,
    expand_command_arguments,
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
        let action = request.action.as_ref().clone();
        let Some(program) = action.program().cloned() else {
            let reason = action
                .unavailable_reason()
                .unwrap_or("Custom actions on Windows require native process-tree isolation")
                .to_owned();
            sink(ActionRunEvent::Failed(reason));
            return Rc::new(|| {});
        };

        let cancelled = Arc::new(AtomicBool::new(false));
        let cancel_flag = cancelled.clone();
        let sink_clone = sink.clone();

        let (exe, args) = match &program {
            ActionProgram::Script {
                interpreter,
                interpreter_arguments,
                script,
                ..
            } => {
                let mut cmd_args: Vec<std::ffi::OsString> = interpreter_arguments.clone();
                cmd_args.push(script.as_os_str().to_os_string());
                (interpreter.clone(), cmd_args)
            }
            ActionProgram::Command { program: prg, .. } => {
                match expand_command_arguments(&program, &request.inputs, &request.parent) {
                    Ok(args) => (prg.clone(), args),
                    Err(e) => {
                        sink(ActionRunEvent::Failed(e));
                        return Rc::new(|| {});
                    }
                }
            }
        };

        let limits = strata_win_shell::job::JobLimits {
            memory_limit_bytes: Some(1024 * 1024 * 1024),
            active_process_limit: Some(16),
            kill_on_job_close: true,
            restrict_ui: false,
        };

        std::thread::spawn(move || {
            let arg_refs: Vec<&std::ffi::OsStr> = args.iter().map(|a| a.as_os_str()).collect();
            let result = strata_win_shell::job::run_in_job(
                Path::new(&exe),
                &arg_refs,
                None,
                &limits,
                Duration::from_secs(300),
                Some(&cancel_flag),
            );

            match result {
                Ok(output) => {
                    let log = String::from_utf8_lossy(&output.stdout).to_string();
                    sink_clone(ActionRunEvent::Exited {
                        code: Some(output.exit_code as i32),
                        signal: None,
                        log,
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {
                    sink_clone(ActionRunEvent::Failed("Action cancelled".to_string()));
                }
                Err(e) => {
                    sink_clone(ActionRunEvent::Failed(format!(
                        "Action failed to start: {e}"
                    )));
                }
            }
        });

        Rc::new(move || {
            cancelled.store(true, Ordering::Relaxed);
        })
    }
}
