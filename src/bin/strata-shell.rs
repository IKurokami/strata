// SPDX-License-Identifier: MIT
#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
#[path = "strata-shell/context_menu.rs"]
mod context_menu;

#[cfg(windows)]
fn main() -> std::process::ExitCode {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    if let Ok(executable) = std::env::current_exe()
        && executable
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case("Start-Strata.exe"))
    {
        return launch_portable(&executable, &arguments);
    }
    let result = match arguments.as_slice() {
        [mode] if mode == "--version" => {
            println!("strata-shell {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        [mode, folder] if mode == "--open" => {
            use std::os::windows::ffi::OsStrExt;
            let request = std::env::current_dir().ok().and_then(|cwd| {
                serde_json::to_vec(&serde_json::json!({"Open": {
                    "cwd": cwd.as_os_str().encode_wide().collect::<Vec<_>>(),
                    "arguments": [folder.encode_wide().collect::<Vec<_>>()]
                }}))
                .ok()
            });
            strata_win_shell::file_manager::open_with_request(folder, request.as_deref())
        }
        [mode, file] if mode == "--launch" || mode == "--choose" => {
            match strata_win_shell::launch::open(std::path::Path::new(file), mode == "--choose") {
                Ok(()) => Ok(()),
                Err(error) => {
                    eprintln!("{error}");
                    return std::process::ExitCode::FAILURE;
                }
            }
        }
        [mode] if mode == "--context-menu-model" => context_menu::serve(),
        [mode] if mode == "--watch" => strata_win_shell::file_manager::watch(),
        [mode, flag, target] if mode == "--unregister" && flag == "--target" => {
            strata_win_shell::file_manager::disable(std::path::Path::new(target))
        }
        _ => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Invalid shell helper arguments",
        )),
    };
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Strata shell helper: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(windows)]
fn launch_portable(
    executable: &std::path::Path,
    arguments: &[std::ffi::OsString],
) -> std::process::ExitCode {
    let Some(root) = executable.parent() else {
        return std::process::ExitCode::FAILURE;
    };
    let mut command = std::process::Command::new(root.join("bin/strata.exe"));
    command.args(arguments);
    // CLI invocations retain redirected output and an observable exit status.
    // Ordinary desktop launches leave no extra launcher process resident.
    if arguments
        .iter()
        .any(|argument| argument == "--version" || argument == "--quit")
    {
        match command.status() {
            Ok(status) if status.success() => std::process::ExitCode::SUCCESS,
            _ => std::process::ExitCode::FAILURE,
        }
    } else {
        match command.spawn() {
            Ok(_) => std::process::ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("Strata: {error}");
                std::process::ExitCode::FAILURE
            }
        }
    }
}

#[cfg(not(windows))]
fn main() {}
