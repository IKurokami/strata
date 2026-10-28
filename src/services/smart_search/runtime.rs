// SPDX-License-Identifier: MIT

use base64::Engine;
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

pub(super) struct Embedder {
    child: Child,
    input: ChildStdin,
    replies: mpsc::Receiver<Result<Value, String>>,
    sequence: u64,
    last_used: Instant,
    #[cfg(windows)]
    _job: strata_win_shell::job::JobObject,
}

pub(super) fn executable() -> Option<PathBuf> {
    let name = if cfg!(windows) { "digup.exe" } else { "digup" };
    let parent = std::env::current_exe().ok()?.parent()?.to_owned();
    [
        parent.join("smart-search").join(name),
        parent.join("../embedding-runtime").join(name),
        parent.join("../../embedding-runtime").join(name),
        parent.join("../lib/strata/smart-search").join(name),
        parent.join("../libexec/strata/smart-search").join(name),
    ]
    .into_iter()
    .find(|path| path.is_file())
}

impl Embedder {
    pub fn start(
        data: &Path,
        artifacts: &Path,
        text_only: bool,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        super::model::verify(data, cancelled)?;
        let executable = executable().ok_or("Smart Search runtime is not installed")?;
        let mut command = Command::new(&executable);
        command.arg("worker");
        if text_only {
            command.arg("--text-only");
        }
        command
            .env("DIGUP_MODELS", super::model::folder(data))
            .env("DIGUP_DEVICE", "cpu");
        command
            .env("DIGUP_THREADS", "2")
            .env("GGML_N_THREADS", "2")
            .env("OMP_NUM_THREADS", "2");
        command
            .current_dir(artifacts)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000 | 0x0000_4000);
        }
        #[cfg(target_os = "linux")]
        let mut command = isolated(
            command,
            &executable,
            &super::model::folder(data),
            artifacts,
            text_only,
        )?;
        let mut child = command.spawn().map_err(|error| error.to_string())?;
        #[cfg(windows)]
        let job =
            match strata_win_shell::job::JobObject::create(&strata_win_shell::job::JobLimits {
                memory_limit_bytes: Some(if text_only {
                    768 * 1024 * 1024
                } else {
                    2 * 1024 * 1024 * 1024
                }),
                active_process_limit: Some(1),
                kill_on_job_close: true,
                restrict_ui: true,
            })
            .and_then(|job| {
                job.assign_child(&child)?;
                Ok(job)
            }) {
                Ok(job) => job,
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(error.to_string());
                }
            };
        let (Some(input), Some(output)) = (child.stdin.take(), child.stdout.take()) else {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Embedding worker pipes are unavailable".into());
        };
        let (send, replies) = mpsc::sync_channel(1);
        let reader = std::thread::Builder::new()
            .name("embedding-replies".into())
            .spawn(move || {
                let mut output = BufReader::new(output);
                loop {
                    let mut line = Vec::new();
                    let result = (&mut output)
                        .take(1024 * 1024 + 1)
                        .read_until(b'\n', &mut line);
                    let value = match result {
                        Ok(0) => break,
                        Ok(_) if line.len() <= 1024 * 1024 => {
                            serde_json::from_slice(&line).map_err(|error| error.to_string())
                        }
                        Ok(_) => Err("Embedding worker response exceeds its limit".into()),
                        Err(error) => Err(error.to_string()),
                    };
                    let failed = value.is_err();
                    if send.send(value).is_err() || failed {
                        break;
                    }
                }
            });
        if let Err(error) = reader {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error.to_string());
        }
        let mut worker = Self {
            child,
            input,
            replies,
            sequence: 0,
            last_used: Instant::now(),
            #[cfg(windows)]
            _job: job,
        };
        let ready = worker.receive(Duration::from_secs(90), cancelled)?;
        let info = &ready["info"];
        if ready["ready"] != true
            || info["dtype"] != "text-q8_0+mmproj-q8_0"
            || info["dim"] != 768
            || info["revision"] != super::model::REVISION
            || info["runtime"] != "llama.cpp-vectors1"
            || info["model"] != "google/embeddinggemma-2"
            || info["text_only"] != text_only
        {
            return Err("Embedding worker fingerprint mismatch".into());
        }
        Ok(worker)
    }

    pub fn idle(&self) -> bool {
        self.last_used.elapsed() > Duration::from_secs(300)
    }

    pub fn embed(
        &mut self,
        operation: &str,
        inputs: &[String],
        cancelled: &impl Fn() -> bool,
    ) -> Result<Vec<Option<Vec<f32>>>, String> {
        if inputs.is_empty()
            || inputs.len() > 8
            || inputs.iter().any(|input| input.len() > 32 * 1024)
        {
            return Err("Embedding request exceeds its budget".into());
        }
        self.sequence = self.sequence.wrapping_add(1);
        let request = json!({"id":self.sequence,"op":operation,"inputs":inputs,"budget":280});
        serde_json::to_writer(&mut self.input, &request).map_err(|error| error.to_string())?;
        self.input
            .write_all(b"\n")
            .and_then(|()| self.input.flush())
            .map_err(|error| error.to_string())?;
        let response = self.receive(Duration::from_secs(60), cancelled)?;
        let vectors = decode_vectors(&response, self.sequence, inputs.len())?;
        self.last_used = Instant::now();
        Ok(vectors)
    }

    fn receive(
        &mut self,
        timeout: Duration,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Value, String> {
        let started = Instant::now();
        let result = loop {
            if cancelled() {
                break Err("Embedding request cancelled".into());
            }
            if started.elapsed() > timeout {
                break Err("Embedding worker timed out".into());
            }
            match self.replies.recv_timeout(Duration::from_millis(20)) {
                Ok(reply) => break reply,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    break Err("Embedding worker exited".into());
                }
            }
        };
        if result.is_err() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        result
    }
}

impl Drop for Embedder {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(target_os = "linux")]
fn isolated(
    _original: Command,
    executable: &Path,
    models: &Path,
    artifacts: &Path,
    text_only: bool,
) -> Result<Command, String> {
    let mut command = Command::new("bwrap");
    command.args([
        "--die-with-parent",
        "--unshare-net",
        "--unshare-pid",
        "--new-session",
        "--proc",
        "/proc",
        "--dev",
        "/dev",
        "--tmpfs",
        "/tmp",
    ]);
    for path in [
        Path::new("/usr"),
        Path::new("/lib"),
        Path::new("/lib64"),
        executable
            .parent()
            .ok_or("Invalid embedding runtime path")?,
        models,
        artifacts,
    ] {
        if path.exists() {
            command.arg("--ro-bind").arg(path).arg(path);
        }
    }
    command
        .arg("--chdir")
        .arg(artifacts)
        .arg("--")
        .arg(executable)
        .arg("worker");
    if text_only {
        command.arg("--text-only");
    }
    command
        .env("DIGUP_MODELS", models)
        .env("DIGUP_DEVICE", "cpu")
        .env("DIGUP_THREADS", "2")
        .env("GGML_N_THREADS", "2")
        .env("OMP_NUM_THREADS", "2")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    Ok(command)
}

fn decode_vectors(
    response: &Value,
    sequence: u64,
    count: usize,
) -> Result<Vec<Option<Vec<f32>>>, String> {
    if response["id"] != sequence
        || response["ok"] != true
        || response["dim"] != 768
        || response["count"] != count
    {
        return Err("Embedding worker returned an invalid response".into());
    }
    if response["errors"]
        .as_array()
        .is_none_or(|errors| errors.len() != count)
    {
        return Err("Embedding worker returned an invalid error map".into());
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(
            response["vectors"]
                .as_str()
                .ok_or("Embedding worker returned no vectors")?,
        )
        .map_err(|error| error.to_string())?;
    if bytes.len() != count * 768 * 4 {
        return Err("Embedding vector dimensions do not match".into());
    }
    let mut vectors = Vec::with_capacity(count);
    for (index, row) in bytes.as_chunks::<{ 768 * 4 }>().0.iter().enumerate() {
        if !response["errors"][index].is_null() {
            vectors.push(None);
            continue;
        }
        let mut vector: Vec<f32> = row
            .as_chunks::<4>()
            .0
            .iter()
            .map(|bytes| f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
            .collect();
        let length = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
        if !length.is_finite() || length < 1e-8 {
            return Err("Embedding worker returned a nonfinite vector".into());
        }
        for value in &mut vector {
            *value /= length;
        }
        vectors.push(Some(vector));
    }
    Ok(vectors)
}

#[cfg(test)]
mod tests;
