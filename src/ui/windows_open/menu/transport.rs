use std::{
    io::{self, BufRead, BufReader, Read, Write},
    os::windows::ffi::OsStrExt,
    path::PathBuf,
    process::{Child, Command},
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

use futures_channel::oneshot;

use super::Node;

const MAX_FRAME: u64 = 4 * 1024 * 1024;
type Reply = oneshot::Receiver<io::Result<()>>;
type Snapshot = oneshot::Receiver<io::Result<Vec<Node>>>;

enum Request {
    Invoke(u32),
    Cancel,
}

pub(super) struct Session {
    sender: mpsc::Sender<Request>,
    pid: Arc<AtomicU32>,
    completion: Option<Reply>,
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.sender.send(Request::Cancel);
    }
}

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl Session {
    pub(super) fn start(paths: Vec<PathBuf>) -> io::Result<(Self, Snapshot)> {
        let mut command = super::super::helper_command()?;
        command.arg("--context-menu-model");
        Self::start_command(paths, command, Duration::from_secs(10))
    }

    fn start_command(
        paths: Vec<PathBuf>,
        command: Command,
        timeout: Duration,
    ) -> io::Result<(Self, Snapshot)> {
        let (sender, receiver) = mpsc::channel();
        let (ready, snapshot) = oneshot::channel();
        let (completed, completion) = oneshot::channel();
        let pid = Arc::new(AtomicU32::new(0));
        let worker_pid = pid.clone();
        std::thread::Builder::new()
            .name("strata-shell-menu".into())
            .spawn(move || {
                let result = run(paths, command, timeout, receiver, ready, worker_pid);
                let _ = completed.send(result);
            })?;
        Ok((
            Self {
                sender,
                pid,
                completion: Some(completion),
            },
            snapshot,
        ))
    }

    pub(super) fn invoke(&mut self, command: u32) -> Option<Reply> {
        let completion = self.completion.take()?;
        strata_win_shell::context_menu::allow_foreground(self.pid.load(Ordering::Acquire));
        self.sender.send(Request::Invoke(command)).ok()?;
        Some(completion)
    }
}

fn run(
    paths: Vec<PathBuf>,
    mut command: Command,
    timeout: Duration,
    requests: mpsc::Receiver<Request>,
    ready: oneshot::Sender<io::Result<Vec<Node>>>,
    pid: Arc<AtomicU32>,
) -> io::Result<()> {
    let paths: Vec<Vec<u16>> = paths
        .iter()
        .map(|path| path.as_os_str().encode_wide().collect())
        .collect();
    let mut selection = serde_json::to_vec(&paths).map_err(io::Error::other)?;
    if selection.len() as u64 >= MAX_FRAME {
        return Err(io::Error::other("Shell selection limit exceeded"));
    }
    selection.push(b'\n');
    command
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    let mut child = ChildGuard(command.spawn()?);
    pid.store(child.0.id(), Ordering::Release);
    let mut input = child
        .0
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("Missing Shell input"))?;
    let output = child
        .0
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("Missing Shell output"))?;
    let (input_sender, input_receiver) = mpsc::channel();
    std::thread::Builder::new()
        .name("strata-shell-selection".into())
        .spawn(move || {
            let result = input.write_all(&selection).and_then(|_| input.flush());
            let _ = input_sender.send(result.map(|_| input));
        })?;
    let mut input = None;
    let (read_sender, read_receiver) = mpsc::channel();
    std::thread::Builder::new()
        .name("strata-shell-snapshot".into())
        .spawn(move || {
            let mut output = BufReader::new(output);
            let result = read_snapshot(&mut output);
            let _ = read_sender.send(result);
            // Extensions may write diagnostics during invocation too. Closing this
            // pipe after the snapshot can fail their command with a broken pipe.
            let _ = io::copy(&mut output, &mut io::sink());
        })?;
    let mut ready = Some(ready);
    let deadline = Instant::now() + timeout;
    let mut received = false;
    let mut invoked = false;
    let mut pending = None;
    loop {
        if input.is_none()
            && let Ok(writer) = input_receiver.try_recv()
        {
            input = Some(writer?);
        }
        if let Ok(result) = read_receiver.try_recv() {
            let success = result.is_ok();
            if let Some(ready) = ready.take() {
                let _ = ready.send(result);
            }
            if !success {
                return Err(io::Error::other("Invalid Shell response"));
            }
            received = true;
        }
        match requests.recv_timeout(Duration::from_millis(20)) {
            Ok(Request::Invoke(command)) if !invoked => {
                pending = Some(command);
            }
            Ok(Request::Cancel) | Err(mpsc::RecvTimeoutError::Disconnected)
                if !invoked && pending.is_none() =>
            {
                return Ok(());
            }
            _ => {}
        }
        if let Some(writer) = input.as_mut()
            && let Some(command) = pending.take()
        {
            writeln!(writer, "{command}")?;
            writer.flush()?;
            invoked = true;
        }
        if let Some(status) = child.0.try_wait()? {
            return if status.success() {
                Ok(())
            } else {
                Err(io::Error::other("Shell command failed"))
            };
        }
        if (!received || input.is_none()) && !invoked && Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Shell menu enumeration timed out",
            ));
        }
        // Invoked commands may own a legitimate long-lived native dialog.
        // Cancellation only kills enumeration/idle sessions, never that dialog.
        if invoked || pending.is_some() {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

fn read_snapshot(output: impl Read) -> io::Result<Vec<Node>> {
    let mut reader = BufReader::new(output).take(MAX_FRAME);
    loop {
        let mut line = Vec::new();
        if reader.read_until(b'\n', &mut line)? == 0 {
            return Err(io::Error::other("Shell snapshot unavailable"));
        }
        if !line.ends_with(b"\n") {
            return Err(io::Error::other("Truncated Shell snapshot"));
        }
        if let Some(payload) = line.strip_prefix(b"STRATA_SHELL_MENU_V1 ") {
            let nodes: Vec<Node> = serde_json::from_slice(payload).map_err(io::Error::other)?;
            super::validate(&nodes)?;
            return Ok(nodes);
        }
    }
}

#[cfg(test)]
mod tests;
