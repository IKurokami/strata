// SPDX-License-Identifier: MIT
use super::{
    Registration, lock,
    registry::{self, Value},
};
use std::{
    io, thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use windows::Win32::System::Registry::REG_BINARY;

fn now() -> io::Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|t| t.as_secs())
        .map_err(io::Error::other)
}

impl Registration {
    fn scope(&self) -> io::Result<Option<String>> {
        let Some(value) = registry::get(&self.state, "Activation")? else {
            return Ok(None);
        };
        if value.kind != REG_BINARY || value.data.len() < 4 {
            return Ok(None);
        }
        let count =
            u32::from_le_bytes(value.data[..4].try_into().expect("activation header")) as usize;
        if count == 0 || count > 32 * 1024 || count > value.data.len() - 4 {
            return Ok(None);
        }
        Ok(std::str::from_utf8(&value.data[4..4 + count])
            .ok()
            .map(str::to_owned))
    }
    fn ready(&self) -> io::Result<bool> {
        Ok(self
            .scope()?
            .is_some_and(|s| crate::activation::available(&s)))
    }
    fn launching(&self) -> io::Result<bool> {
        let Some(value) = registry::get(&self.state, "Launching")? else {
            return Ok(false);
        };
        if value.kind != REG_BINARY || value.data.len() != 20 {
            return Ok(false);
        }
        let pid = u32::from_le_bytes(value.data[..4].try_into().expect("pid"));
        let started = u64::from_le_bytes(value.data[4..12].try_into().expect("creation time"));
        let at = u64::from_le_bytes(value.data[12..].try_into().expect("launch time"));
        Ok(now()?.checked_sub(at).is_some_and(|age| age < 30)
            && crate::restart::live_identity(pid)? == Some(started))
    }
    fn remember(&self, pid: u32) -> io::Result<()> {
        if let Some(started) = crate::restart::live_identity(pid)? {
            let mut data = pid.to_le_bytes().to_vec();
            data.extend_from_slice(&started.to_le_bytes());
            data.extend_from_slice(&now()?.to_le_bytes());
            registry::set(
                &self.state,
                "Launching",
                Some(&Value {
                    kind: REG_BINARY,
                    data,
                }),
            )?;
        }
        Ok(())
    }

    pub(super) fn launch_once(
        &self,
        request: Option<&[u8]>,
        spawn: impl FnOnce() -> io::Result<u32>,
    ) -> io::Result<()> {
        if self.forward_running(request) {
            return Ok(());
        }
        let _guard = lock::launch(&self.state)?;
        if self.forward_running(request) {
            return Ok(());
        }
        let pending = self.launching()?;
        if !pending {
            self.remember(spawn()?)?;
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if self.ready()? {
                // The first request travels in the spawned process's argv. Only
                // queued later launches send another request through the pipe.
                if !pending || self.forward_running(request) {
                    return Ok(());
                }
            }
            if !self.launching()? {
                return Err(io::Error::new(
                    io::ErrorKind::ConnectionAborted,
                    "Strata exited before activation became available",
                ));
            }
            thread::sleep(Duration::from_millis(20));
        }
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "Strata activation is still starting",
        ))
    }
}

#[cfg(test)]
mod tests;
