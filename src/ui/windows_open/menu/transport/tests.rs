use super::*;

fn frame() -> String {
    format!(
        "STRATA_SHELL_MENU_V1 {}\n",
        serde_json::json!([{
            "label": "&Editor_工具", "command": 17, "enabled": true,
            "checked": false, "separator": false, "submenu": false, "children": []
        }])
    )
}

fn fixture(mode: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().expect("Shell menu fixture"));
    command
        .args([
            "--exact",
            "ui::windows_open::menu::transport::tests::fixture_helper",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("STRATA_SHELL_MENU_FIXTURE", mode);
    command
}

fn await_result<T>(mut receiver: oneshot::Receiver<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(result) = receiver.try_recv().expect("worker reply channel") {
            return result;
        }
        assert!(Instant::now() < deadline, "worker reply timed out");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn protocol_accepts_extension_diagnostics_but_rejects_bad_frames() {
    let input = [
        b"extension diagnostic \xff\n".as_slice(),
        frame().as_bytes(),
    ]
    .concat();
    let nodes = read_snapshot(input.as_slice()).expect("Shell menu fixture");
    assert_eq!(nodes[0].command, Some(17));
    assert_eq!(nodes[0].label, "&Editor_工具");
    for bad in [
        frame().trim_end().to_owned(),
        frame().replace("17", "99999"),
        "STRATA_SHELL_MENU_V1 invalid\n".into(),
        "noise\n".repeat(MAX_FRAME as usize / 6 + 1),
    ] {
        assert!(read_snapshot(bad.as_bytes()).is_err());
    }
}

#[test]
fn cancellation_reaps_a_helper_even_when_selection_write_is_blocked() {
    let paths = vec![PathBuf::from(r"C:\").join("x".repeat(128)); 128];
    let (mut session, _snapshot) =
        Session::start_command(paths, fixture("hang"), Duration::from_secs(5))
            .expect("Shell menu fixture");
    let deadline = Instant::now() + Duration::from_secs(5);
    while session.pid.load(Ordering::Acquire) == 0 {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    let completion = session.completion.take().expect("Shell menu fixture");
    drop(session);
    assert!(await_result(completion).is_ok());
}

#[test]
fn hung_enumeration_times_out_without_waiting_on_gtk() {
    let (mut session, _snapshot) = Session::start_command(
        vec![PathBuf::from(r"C:\test.txt")],
        fixture("hang"),
        Duration::from_millis(400),
    )
    .expect("Shell menu fixture");
    let completion = session.completion.take().expect("Shell menu fixture");
    assert_eq!(
        await_result(completion)
            .expect_err("expected Shell menu rejection")
            .kind(),
        io::ErrorKind::TimedOut
    );
}

#[test]
fn invocation_uses_the_retained_session_and_survives_menu_closure() {
    let directory = tempfile::tempdir().expect("Shell menu fixture");
    let marker = directory.path().join("selected-command.txt");
    let (mut session, snapshot) = Session::start_command(
        vec![marker.clone()],
        fixture("invoke"),
        Duration::from_secs(5),
    )
    .expect("Shell menu fixture");
    assert_eq!(
        await_result(snapshot).expect("Shell menu fixture")[0].command,
        Some(17)
    );
    let pid = session.pid.load(Ordering::Acquire);
    let completion = session.invoke(17).expect("Shell menu fixture");
    assert!(session.invoke(18).is_none());
    drop(session);
    assert!(await_result(completion).is_ok());
    assert_eq!(
        std::fs::read_to_string(marker).expect("Shell menu fixture"),
        format!("17:{pid}")
    );
}

#[test]
fn failed_invocation_keeps_the_native_error_for_diagnostics() {
    let (mut session, snapshot) = Session::start_command(
        vec![PathBuf::from(r"C:\fixture.txt")],
        fixture("fail"),
        Duration::from_secs(5),
    )
    .expect("start failure fixture");
    await_result(snapshot).expect("receive commands");
    let completion = session.invoke(17).expect("invoke fixture");
    let error = await_result(completion).expect_err("native command failed");
    assert!(
        error
            .to_string()
            .contains("fixture application not found (0x80070483)"),
        "{error}"
    );
}

#[test]
fn fixture_helper() {
    let Ok(mode) = std::env::var("STRATA_SHELL_MENU_FIXTURE") else {
        return;
    };
    if mode == "hang" {
        std::thread::sleep(Duration::from_secs(30));
        return;
    }
    let mut input = BufReader::new(io::stdin());
    let mut selection = String::new();
    input.read_line(&mut selection).expect("Shell menu fixture");
    let paths: Vec<Vec<u16>> = serde_json::from_str(&selection).expect("Shell menu fixture");
    let path = PathBuf::from(
        <std::ffi::OsString as std::os::windows::ffi::OsStringExt>::from_wide(&paths[0]),
    );
    print!("\n{}", frame());
    io::stdout().flush().expect("Shell menu fixture");
    let mut command = String::new();
    input.read_line(&mut command).expect("Shell menu fixture");
    if mode == "fail" {
        eprintln!("fixture application not found (0x80070483)");
        std::process::exit(7);
    }
    std::thread::sleep(Duration::from_millis(100));
    std::fs::write(path, format!("{}:{}", command.trim(), std::process::id()))
        .expect("Shell menu fixture");
}
