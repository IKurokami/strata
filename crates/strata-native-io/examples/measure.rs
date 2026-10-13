//! Repeatable, opt-in local benchmark. No privileged NTFS APIs and no
//! absolute pass/fail latency threshold (hosted runners are too noisy).
use std::{
    env, io,
    path::Path,
    thread,
    time::{Duration, Instant},
};

use strata_native_io::{ListingEvent, ListingService};

// Enough warm samples for a meaningful observed p99 instead of a single-digit estimate.
const RUNS: usize = 100;
const TIMEOUT: Duration = Duration::from_secs(60);

struct Measurement {
    first_batch_ms: f64,
    first_batch_items: usize,
    complete_ms: f64,
    item_count: usize,
}

fn scan_once(service: &ListingService, path: &Path) -> io::Result<Measurement> {
    let start = Instant::now();
    let generation = service.request(path);
    let mut first_batch_ms = None;
    let mut first_batch_items = 0;
    let mut item_count = 0;

    loop {
        match service.try_recv() {
            Ok(ListingEvent::Batch {
                generation: current,
                entries,
            }) if current == generation => {
                if first_batch_ms.is_none() {
                    first_batch_ms = Some(start.elapsed().as_secs_f64() * 1_000.0);
                    first_batch_items = entries.len();
                }
                item_count += entries.len();
            }
            Ok(ListingEvent::Finished {
                generation: current,
                error,
            }) if current == generation => {
                if let Some(error) = error {
                    return Err(io::Error::other(error));
                }
                let complete_ms = start.elapsed().as_secs_f64() * 1_000.0;
                return Ok(Measurement {
                    first_batch_ms: first_batch_ms.unwrap_or(complete_ms),
                    first_batch_items,
                    complete_ms,
                    item_count,
                });
            }
            Ok(_) => {} // Discard stale generations, never block on their work.
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                thread::sleep(Duration::from_micros(250));
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                return Err(io::Error::other("directory worker disconnected"));
            }
        }

        if start.elapsed() > TIMEOUT {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "listing did not complete within 60 seconds",
            ));
        }
    }
}

fn percentile(samples: &mut [f64], percent: usize) -> f64 {
    samples.sort_by(f64::total_cmp);
    let index = (samples.len() * percent).div_ceil(100).saturating_sub(1);
    samples[index.min(samples.len() - 1)]
}

fn main() -> io::Result<()> {
    let path = env::args_os()
        .nth(1)
        .map(std::path::PathBuf::from)
        .unwrap_or(env::current_dir()?);

    let worker_start = Instant::now();
    let service = ListingService::new();
    let worker_setup_ms = worker_start.elapsed().as_secs_f64() * 1_000.0;
    let mut first_times = Vec::with_capacity(RUNS);
    let mut full_times = Vec::with_capacity(RUNS);
    let mut last_count = 0;

    // The first pass includes filesystem/cache warmup; subsequent passes
    // represent warm local listing performance, not cold-boot performance.
    let first = scan_once(&service, &path)?;
    println!(
        "Initial pass: {:.2}ms first {} entries; {:.2}ms complete; {} entries",
        first.first_batch_ms, first.first_batch_items, first.complete_ms, first.item_count
    );

    for _ in 0..RUNS {
        let measurement = scan_once(&service, &path)?;
        first_times.push(measurement.first_batch_ms);
        full_times.push(measurement.complete_ms);
        last_count = measurement.item_count;
    }

    println!("Path: {}", path.display());
    println!("Native listing worker setup: {worker_setup_ms:.2}ms (not GTK startup)");
    println!("Warm samples: {RUNS}; last item count: {last_count}");
    println!(
        "First batch: p50 {:.2}ms / p95 {:.2}ms / p99 {:.2}ms",
        percentile(&mut first_times.clone(), 50),
        percentile(&mut first_times.clone(), 95),
        percentile(&mut first_times, 99)
    );
    println!(
        "All names:   p50 {:.2}ms / p95 {:.2}ms / p99 {:.2}ms",
        percentile(&mut full_times.clone(), 50),
        percentile(&mut full_times.clone(), 95),
        percentile(&mut full_times, 99)
    );
    Ok(())
}
