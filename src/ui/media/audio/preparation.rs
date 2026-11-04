// SPDX-License-Identifier: MIT

use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};

static INITIALIZING: AtomicBool = AtomicBool::new(false);

struct SetupSlot;

impl Drop for SetupSlot {
    fn drop(&mut self) {
        INITIALIZING.store(false, Ordering::Release);
    }
}

// Only GStreamer objects cross threads; PcmOutput's UI state stays thread-bound.
pub(in crate::ui::media) struct PreparedOutput {
    output: Option<(gst::Pipeline, AppSrc, gst::Element, gst::Element)>,
    max_bytes: u64,
    max_time_ns: u64,
}

impl PreparedOutput {
    pub(super) fn prepare(muted: bool, volume: f64, lookahead: Duration) -> Result<Self, String> {
        let mut output = PcmOutput::new(muted, volume, lookahead)?;
        let prepared = Self {
            output: Some((
                output.pipeline.clone(),
                output.source.clone(),
                output.sink.clone(),
                output.volume.clone(),
            )),
            max_bytes: output.max_bytes,
            max_time_ns: output.max_time_ns,
        };
        output.shutdown_on_drop = false;
        Ok(prepared)
    }

    pub(in crate::ui::media) fn finish(mut self, muted: bool, volume: f64) -> PcmOutput {
        let output = self.take_output().expect("prepared audio consumed once");
        output.set_audio(muted, volume);
        output
    }

    fn take_output(&mut self) -> Option<PcmOutput> {
        let (pipeline, source, sink, gain) = self.output.take()?;
        Some(PcmOutput {
            pipeline,
            source,
            sink,
            volume: gain,
            gain: Cell::new(1.0),
            fade: Cell::new(1.0),
            max_bytes: self.max_bytes,
            max_time_ns: self.max_time_ns,
            frames: Cell::new(0),
            finished: Cell::new(false),
            failure: RefCell::new(None),
            shutdown_on_drop: true,
            _thread: PhantomData,
        })
    }
}

impl Drop for PreparedOutput {
    fn drop(&mut self) {
        // Reuse asynchronous shutdown even when an unclaimed result is dropped on GTK.
        drop(self.take_output());
    }
}

pub(in crate::ui::media) struct AudioPreparation {
    cancelled: Arc<AtomicBool>,
    receiver: mpsc::Receiver<Result<PreparedOutput, String>>,
}

impl AudioPreparation {
    pub(in crate::ui::media) fn start(
        muted: bool,
        volume: f64,
        lookahead: Duration,
    ) -> Result<Option<Self>, String> {
        Self::start_with(move || PreparedOutput::prepare(muted, volume, lookahead))
    }

    pub(super) fn start_with(
        build: impl FnOnce() -> Result<PreparedOutput, String> + Send + 'static,
    ) -> Result<Option<Self>, String> {
        if INITIALIZING
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Ok(None);
        }
        let slot = SetupSlot;
        let cancelled = Arc::new(AtomicBool::new(false));
        let stop = cancelled.clone();
        let (sender, receiver) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("pcm-prepare".into())
            .spawn(move || {
                let _slot = slot;
                if !stop.load(Ordering::Acquire) {
                    let started = std::time::Instant::now();
                    let result = build();
                    tracing::debug!(
                        elapsed_ms = started.elapsed().as_millis() as u64,
                        "media audio preparation finished off GTK"
                    );
                    if !stop.load(Ordering::Acquire) {
                        let _ = sender.send(result);
                    }
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Some(Self {
            cancelled,
            receiver,
        }))
    }

    pub(in crate::ui::media) fn poll(&self) -> Result<Option<PreparedOutput>, String> {
        match self.receiver.try_recv() {
            Ok(result) => result.map(Some),
            Err(mpsc::TryRecvError::Empty) => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }
}

impl Drop for AudioPreparation {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

#[cfg(test)]
mod tests;
