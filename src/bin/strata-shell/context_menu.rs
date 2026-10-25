use std::{
    ffi::OsString,
    io::{self, BufRead, BufReader, Read, Write},
    os::windows::ffi::OsStringExt,
    path::PathBuf,
    sync::mpsc,
    time::Duration,
};

const MAX_FRAME: u64 = 4 * 1024 * 1024;

fn export(entry: &strata_win_shell::context_menu::Entry) -> serde_json::Value {
    serde_json::json!({
        "label": entry.label, "command": entry.command, "verb": entry.verb, "enabled": entry.enabled,
        "checked": entry.checked, "separator": entry.separator, "submenu": entry.submenu,
        "children": entry.children.iter().map(export).collect::<Vec<_>>()
    })
}

pub(super) fn serve() -> io::Result<()> {
    let mut input = BufReader::new(io::stdin());
    let mut selection = String::new();
    Read::by_ref(&mut input)
        .take(MAX_FRAME)
        .read_line(&mut selection)?;
    if !selection.ends_with('\n') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid Shell selection frame",
        ));
    }
    let paths: Vec<Vec<u16>> = serde_json::from_str(&selection).map_err(io::Error::other)?;
    let paths: Vec<_> = paths
        .iter()
        .map(|path| PathBuf::from(OsString::from_wide(path)))
        .collect();
    let mut session = strata_win_shell::context_menu::Session::new(&paths)?;
    let entries = session.snapshot()?;
    let frame = serde_json::to_vec(&entries.iter().map(export).collect::<Vec<_>>())
        .map_err(io::Error::other)?;
    if frame.len() as u64 > MAX_FRAME - 64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Shell menu frame limit exceeded",
        ));
    }
    {
        let mut output = io::stdout().lock();
        output.write_all(b"STRATA_SHELL_MENU_V1 ")?;
        output.write_all(&frame)?;
        output.write_all(b"\n")?;
        output.flush()?;
    }
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut command = String::new();
        let command = match input.take(64).read_line(&mut command) {
            Ok(_) => command.trim().parse::<u32>().ok(),
            Err(_) => None,
        };
        let _ = sender.send(command);
    });
    loop {
        session.pump_messages();
        match receiver.recv_timeout(Duration::from_millis(10)) {
            Ok(Some(command)) => return session.invoke(command),
            Ok(None) | Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
}
