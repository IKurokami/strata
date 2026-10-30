// SPDX-License-Identifier: MIT
use super::super::tests::Fixture;
use super::*;
use std::sync::{
    Arc, Barrier, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

#[test]
fn cold_activation_burst_starts_once_and_warm_requests_reach_the_primary_queue() {
    let fixture = Fixture::new();
    let reg = &fixture.registration;
    reg.enable(&fixture.executable(), &fixture.proxy(), "Strata")
        .unwrap();
    let scope = format!("{}-launch", fixture.key);
    reg.set_activation(Some((&scope, b"activate"))).unwrap();
    let start = Arc::new(Barrier::new(100));
    let spawned = Arc::new(AtomicUsize::new(0));
    let received = Arc::new(Mutex::new(Vec::new()));
    let stop = Arc::new(AtomicBool::new(false));
    let server = Arc::new(Mutex::new(None));
    let mut workers = Vec::new();
    for _ in 0..100 {
        let (reg, start, spawned, server, scope, received, stop) = (
            reg.clone(),
            start.clone(),
            spawned.clone(),
            server.clone(),
            scope.clone(),
            received.clone(),
            stop.clone(),
        );
        workers.push(thread::spawn(move || {
            start.wait();
            let began = Instant::now();
            reg.launch_once(None, || {
                spawned.fetch_add(1, Ordering::Relaxed);
                let server = server.clone();
                *server.lock().unwrap() = Some(thread::spawn(move || {
                    thread::sleep(Duration::from_millis(40));
                    let server = crate::activation::Server::bind(&scope).unwrap();
                    while !stop.load(Ordering::Acquire) {
                        while let Some(message) = server.receive() {
                            received.lock().unwrap().push(message);
                        }
                        thread::sleep(Duration::from_millis(1));
                    }
                }));
                Ok(std::process::id())
            })
            .unwrap();
            began.elapsed()
        }));
    }
    let mut timings = workers
        .into_iter()
        .map(|w| w.join().expect("launch thread"))
        .collect::<Vec<_>>();
    timings.sort();
    assert_eq!(
        spawned.load(Ordering::Relaxed),
        1,
        "concurrent cold starts share one in-flight process"
    );
    let deadline = Instant::now() + Duration::from_secs(3);
    while received.lock().unwrap().len() < 99 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(
        received.lock().unwrap().as_slice(),
        vec![b"activate".to_vec(); 99],
        "the first activation is delivered through spawn arguments"
    );
    let mut warm = Vec::new();
    for index in 0..30 {
        let message = format!("folder-{index}");
        let began = Instant::now();
        reg.launch_once(Some(message.as_bytes()), || {
            panic!("warm activation must not spawn")
        })
        .unwrap();
        warm.push(began.elapsed());
        let deadline = Instant::now() + Duration::from_secs(3);
        while received.lock().unwrap().len() < 100 + index && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(
            received.lock().unwrap().last().map(Vec::as_slice),
            Some(message.as_bytes())
        );
    }
    warm.sort();
    println!(
        "Component IPC timings (not GUI presentation): cold burst p50={:?} p95={:?} p99={:?}; serial warm p50={:?} p95={:?} p99={:?}",
        timings[49], timings[94], timings[98], warm[14], warm[28], warm[29]
    );
    stop.store(true, Ordering::Release);
    server.lock().unwrap().take().unwrap().join().unwrap();
}

#[test]
fn registry_command_quotes_unicode_unc_and_drive_root_without_a_shell() {
    use super::super::routing::command;
    use std::path::Path;
    use windows::{
        Win32::{
            Foundation::{HLOCAL, LocalFree},
            UI::Shell::CommandLineToArgvW,
        },
        core::PCWSTR,
    };
    fn parse(text: &str) -> Vec<String> {
        let wide = text.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
        let mut count = 0;
        // SAFETY: Input is terminated and count points to initialized storage.
        let values = unsafe { CommandLineToArgvW(PCWSTR(wide.as_ptr()), &mut count) };
        assert!(!values.is_null());
        // SAFETY: Windows allocated count valid argument pointers for this call.
        let args = unsafe { std::slice::from_raw_parts(values, count as usize) };
        let result = args
            .iter()
            .map(|value| {
                // SAFETY: Each returned argument is terminated and alive until LocalFree below.
                unsafe { value.to_string() }.expect("valid fixture UTF-16")
            })
            .collect();
        // SAFETY: Release the single block returned by CommandLineToArgvW exactly once.
        unsafe {
            let _ = LocalFree(Some(HLOCAL(values.cast())));
        }
        result
    }
    for path in [
        r"C:\ảnh & ^ % (test) 😀\strata-shell-v2.exe",
        r"\\server\share\folder name\strata-shell-v2.exe",
    ] {
        let text = command(Path::new(path), false)
            .unwrap()
            .os_string()
            .unwrap();
        for folder in [
            r"C:\".to_owned(),
            r"\\server\share\folder & ^ % name 😀\".into(),
            format!("C:\\{}\\", "long-folder-".repeat(30)),
        ] {
            let args = parse(&text.to_string_lossy().replace("%1", &folder));
            assert_eq!(&args[..2], &[path, "--open"]);
            assert_eq!(Path::new(&args[2]), Path::new(&folder));
        }
        let activate = command(Path::new(path), true).unwrap().os_string().unwrap();
        assert_eq!(parse(&activate.to_string_lossy()), vec![path, "--activate"]);
    }
    assert!(command(Path::new("relative.exe"), true).is_err());
    assert!(command(Path::new("C:\\bad\".exe"), true).is_err());
}
