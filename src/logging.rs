// SPDX-License-Identifier: MIT

use std::{
    env,
    io::{self, Write},
    str::FromStr,
};

use tracing_subscriber::{
    filter::{LevelFilter, Targets},
    layer::SubscriberExt,
    util::SubscriberInitExt,
};

// Closed launcher pipes must not turn logging failures into panics at a GLib boundary.
#[derive(Clone, Copy, Debug, Default)]
pub struct PipeSafeWriter<W>(pub W);

struct ApplicationWriter;

impl Write for ApplicationWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        #[cfg(windows)]
        if strata_win_shell::desktop::write_diagnostics(bytes) {
            return Ok(bytes.len());
        }
        PipeSafeWriter(io::stdout()).write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        PipeSafeWriter(io::stdout()).flush()
    }
}

impl<W: Write> Write for PipeSafeWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self.0.write(buf) {
            Ok(n) => Ok(n),
            Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(buf.len()),
            Err(error) => Err(error),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.0.flush() {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
            Err(error) => Err(error),
        }
    }
}

pub fn initialize() {
    let targets = match env::var("RUST_LOG") {
        Ok(var) => Targets::from_str(&var).unwrap_or_default(),
        Err(_) => Targets::default().with_default(LevelFilter::INFO),
    };
    let formatter = tracing_subscriber::fmt().with_writer(|| ApplicationWriter);
    #[cfg(windows)]
    let formatter = formatter.with_ansi(false);
    let subscriber = formatter
        .with_max_level(LevelFilter::TRACE)
        .log_internal_errors(false)
        .finish()
        .with(targets);
    let _ = subscriber.try_init();
}

#[cfg(test)]
mod tests;
