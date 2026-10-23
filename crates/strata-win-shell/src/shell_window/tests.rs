// SPDX-License-Identifier: MIT

use super::*;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use windows::core::{HSTRING, w};

struct Fixture {
    window: HWND,
    folder: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        // SAFETY: This test thread owns the apartment and its hidden window.
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
            .ok()
            .unwrap();
        let folder = std::env::temp_dir().join(format!(
            "strata-shell-selection-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&folder).unwrap();
        // SAFETY: STATIC is a system class; the test exclusively owns this hidden HWND.
        let window = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!("Strata shell test"),
                WS_OVERLAPPED,
                0,
                0,
                100,
                100,
                None,
                None,
                None,
                None,
            )
        }
        .unwrap();
        Self { window, folder }
    }

    fn registered(&self, shell: &IShellWindows, folder: &std::path::Path) -> bool {
        let location = VARIANT::from(BSTR::from_wide(
            &folder.as_os_str().encode_wide().collect::<Vec<_>>(),
        ));
        let mut window = 0;
        // SAFETY: These initialized variants are borrowed only for the synchronous call.
        let found = unsafe {
            shell.FindWindowSW(
                &location,
                &VARIANT::default(),
                SWC_BROWSER,
                &mut window,
                SWFO_NEEDDISPATCH,
            )
        };
        found.is_ok() && window == self.window.0 as isize as i32
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // SAFETY: The test exclusively owns the window and apartment on this thread.
        unsafe {
            let _ = DestroyWindow(self.window);
            CoUninitialize();
        }
        let _ = std::fs::remove_dir_all(&self.folder);
    }
}

fn wait(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(12);
    loop {
        let mut message = MSG::default();
        // SAFETY: Pump this test thread's messages while its COM registration is called.
        unsafe {
            while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        if ready() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "Windows shell selection timed out"
        );
        thread::sleep(Duration::from_millis(4));
    }
}

#[test]
fn chrome_selection_reaches_registered_window_and_navigation_revokes_old_folder() {
    let fixture = Fixture::new();
    let files: Vec<_> = (0..80)
        .map(|index| {
            let file = fixture.folder.join(format!("tiếng Việt 中文 {index}.txt"));
            std::fs::write(&file, b"selection fixture").unwrap();
            file
        })
        .collect();
    // SAFETY: This test owns an initialized STA; ShellWindows is the OS local server.
    let shell: IShellWindows =
        unsafe { CoCreateInstance(&ShellWindows, None, CLSCTX_LOCAL_SERVER) }.unwrap();
    let registration =
        Registration::new(fixture.window.0 as isize, fixture.folder.clone()).unwrap();
    wait(|| match registration.receive() {
        Some(Event::Registered) => true,
        Some(Event::Failed(error)) => panic!("registration failed: {error}"),
        _ => false,
    });
    assert!(
        fixture.registered(&shell, &fixture.folder),
        "never invoke the Shell open API without our own registered target"
    );
    let folder = fixture.folder.clone();
    let targets = files.clone();
    let (send, result) = mpsc::channel();
    let caller = thread::spawn(move || {
        // SAFETY: This worker exclusively owns its STA and the Shell-allocated PIDLs.
        let outcome = unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().unwrap();
            let mut parent = std::ptr::null_mut();
            SHParseDisplayName(
                &HSTRING::from(folder.as_os_str()),
                None,
                &mut parent,
                0,
                None,
            )
            .unwrap();
            let children: Vec<_> = targets
                .iter()
                .map(|target| {
                    let mut child = std::ptr::null_mut();
                    SHParseDisplayName(
                        &HSTRING::from(target.as_os_str()),
                        None,
                        &mut child,
                        0,
                        None,
                    )
                    .unwrap();
                    child.cast_const()
                })
                .collect();
            // Exercise consecutive Chrome-style requests, including changed selections,
            // before the original bulk-selection case.
            let mut outcome = Ok(());
            for index in 0..12 {
                outcome = SHOpenFolderAndSelectItems(parent, Some(&children[index..index + 1]), 0);
                if outcome.is_err() {
                    break;
                }
            }
            if outcome.is_ok() {
                outcome = SHOpenFolderAndSelectItems(parent, Some(&children), 0);
            }
            for child in children {
                CoTaskMemFree(Some(child.cast()));
            }
            CoTaskMemFree(Some(parent.cast()));
            CoUninitialize();
            outcome
        };
        send.send(outcome.map_err(|error| error.to_string()))
            .unwrap();
    });
    let mut selected = false;
    let mut completed = false;
    wait(|| {
        while let Some(event) = registration.receive() {
            match event {
                Event::Selection {
                    folder,
                    files: selected_files,
                } => {
                    assert_eq!(folder, fixture.folder);
                    selected |= selected_files.len() == files.len()
                        && files.iter().all(|file| selected_files.contains(file));
                }
                Event::Failed(error) => panic!("selection failed: {error}"),
                _ => {}
            }
        }
        if let Ok(outcome) = result.try_recv() {
            outcome.unwrap();
            completed = true;
        }
        selected && completed
    });
    caller.join().unwrap();
    let next = fixture.folder.join("next folder");
    std::fs::create_dir(&next).unwrap();
    registration.navigate(next.clone());
    wait(|| matches!(registration.receive(), Some(Event::Registered)));
    assert!(!fixture.registered(&shell, &fixture.folder));
    assert!(fixture.registered(&shell, &next));
    drop(registration);
    wait(|| !fixture.registered(&shell, &next));
}
