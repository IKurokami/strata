// SPDX-License-Identifier: MIT

//! Windows native I/O benchmark comparing native off-thread listing
//! and atomic operations against baseline filesystem operations.

use std::{
    fs::{self, File},
    io::{self, Write},
    path::Path,
    thread,
    time::{Duration, Instant},
};

use strata_native_io::{ListingEvent, ListingService};

const BENCHMARK_ITEMS: usize = 1_000;
const RUNS: usize = 10;

fn create_benchmark_tree(root: &Path, count: usize) -> io::Result<()> {
    fs::create_dir_all(root)?;
    let mut payload = vec![0u8; 1024];
    for (i, byte) in payload.iter_mut().enumerate() {
        *byte = (i % 256) as u8;
    }
    for i in 0..count {
        let file_path = root.join(format!("file_{i:04}.bin"));
        let mut f = File::create(&file_path)?;
        f.write_all(&payload)?;
    }
    Ok(())
}

fn benchmark_std_fs_readdir(root: &Path) -> io::Result<Duration> {
    let start = Instant::now();
    let mut count = 0;
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let _meta = entry.metadata()?;
        count += 1;
    }
    assert_eq!(count, BENCHMARK_ITEMS);
    Ok(start.elapsed())
}

fn benchmark_native_listing(service: &ListingService, root: &Path) -> io::Result<(Duration, Duration, usize)> {
    let start = Instant::now();
    let generation = service.request(root);
    let mut first_batch = None;
    let mut count = 0;

    loop {
        match service.try_recv() {
            Ok(ListingEvent::Batch { generation: current, entries }) if current == generation => {
                if first_batch.is_none() {
                    first_batch = Some(start.elapsed());
                }
                count += entries.len();
            }
            Ok(ListingEvent::Finished { generation: current, error }) if current == generation => {
                if let Some(err) = error {
                    return Err(io::Error::other(err));
                }
                let total = start.elapsed();
                return Ok((first_batch.unwrap_or(total), total, count));
            }
            Ok(_) => {}
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                thread::sleep(Duration::from_micros(100));
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                return Err(io::Error::other("worker disconnected"));
            }
        }
    }
}

fn benchmark_atomic_move_vs_copy_delete(root: &Path) -> io::Result<(Duration, Duration)> {
    let src_dir = root.join("move_src");
    let dst_dir_rename = root.join("move_dst_rename");
    let dst_dir_copy = root.join("move_dst_copy");
    create_benchmark_tree(&src_dir, 100)?;
    fs::create_dir_all(&dst_dir_rename)?;
    fs::create_dir_all(&dst_dir_copy)?;

    // Benchmark atomic rename
    let start_rename = Instant::now();
    for i in 0..100 {
        let src = src_dir.join(format!("file_{i:04}.bin"));
        let dst = dst_dir_rename.join(format!("file_{i:04}.bin"));
        fs::rename(&src, &dst)?;
    }
    let dur_rename = start_rename.elapsed();

    // Benchmark copy + delete fallback
    let start_copy = Instant::now();
    for i in 0..100 {
        let src = dst_dir_rename.join(format!("file_{i:04}.bin"));
        let dst = dst_dir_copy.join(format!("file_{i:04}.bin"));
        fs::copy(&src, &dst)?;
        fs::remove_file(&src)?;
    }
    let dur_copy = start_copy.elapsed();

    Ok((dur_rename, dur_copy))
}

fn main() -> io::Result<()> {
    println!("=== Strata Windows Performance Benchmark ===");
    let temp = std::env::temp_dir().join(format!("strata-bench-{}", std::process::id()));
    let bench_root = temp.join("listing_1000");

    println!("Setting up test fixture: {BENCHMARK_ITEMS} items at {}", bench_root.display());
    create_benchmark_tree(&bench_root, BENCHMARK_ITEMS)?;

    let service = ListingService::new();

    // Warmup
    let _ = benchmark_std_fs_readdir(&bench_root)?;
    let _ = benchmark_native_listing(&service, &bench_root)?;

    println!("\n[1] Directory Listing Benchmark ({BENCHMARK_ITEMS} items, {RUNS} runs average):");
    let mut std_times = Vec::new();
    let mut native_first_times = Vec::new();
    let mut native_total_times = Vec::new();

    for _ in 0..RUNS {
        let std_d = benchmark_std_fs_readdir(&bench_root)?;
        std_times.push(std_d.as_secs_f64() * 1000.0);

        let (first, total, _) = benchmark_native_listing(&service, &bench_root)?;
        native_first_times.push(first.as_secs_f64() * 1000.0);
        native_total_times.push(total.as_secs_f64() * 1000.0);
    }

    let avg_std: f64 = std_times.iter().sum::<f64>() / RUNS as f64;
    let avg_native_first: f64 = native_first_times.iter().sum::<f64>() / RUNS as f64;
    let avg_native_total: f64 = native_total_times.iter().sum::<f64>() / RUNS as f64;

    println!("  Baseline (std::fs::read_dir + metadata):  {avg_std:.2} ms");
    println!("  Strata Native Worker (First batch to UI):  {avg_native_first:.2} ms ({:.1}x faster time-to-first-paint)", avg_std / avg_native_first.max(0.001));
    println!("  Strata Native Worker (Total completion):   {avg_native_total:.2} ms");

    println!("\n[2] Move Operation Benchmark (100 files):");
    let (rename_dur, copy_dur) = benchmark_atomic_move_vs_copy_delete(&temp)?;
    let rename_ms = rename_dur.as_secs_f64() * 1000.0;
    let copy_ms = copy_dur.as_secs_f64() * 1000.0;
    println!("  Atomic Move (Win32 MoveFile / fs::rename): {rename_ms:.2} ms ({:.0} ops/sec)", 100.0 / rename_dur.as_secs_f64());
    println!("  Fallback Copy + Delete:                   {copy_ms:.2} ms ({:.0} ops/sec)", 100.0 / copy_dur.as_secs_f64());
    println!("  Speedup: {:.1}x faster", copy_ms / rename_ms.max(0.001));

    let _ = fs::remove_dir_all(&temp);
    println!("\n=== Benchmark Completed Successfully ===");
    Ok(())
}
