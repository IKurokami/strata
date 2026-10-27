// SPDX-License-Identifier: MIT

use std::{
    io,
    path::PathBuf,
    process::{Command, Output},
};

use gtk::{gio, glib, prelude::*};

pub(super) async fn launch(path: PathBuf, choose: bool) -> io::Result<()> {
    run_helper(
        vec![path],
        if choose { "--choose" } else { "--launch" },
        "Unable to open file",
    )
    .await
}

async fn run_helper(
    paths: Vec<PathBuf>,
    mode: &'static str,
    error_label: &'static str,
) -> io::Result<()> {
    let fallback = crate::i18n::tr(error_label);
    gio::spawn_blocking(move || {
        let mut command = helper_command()?;
        command.arg(mode).args(paths);
        command
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let child = command.spawn()?;
        strata_win_shell::context_menu::allow_foreground(child.id());
        helper_result(child.wait_with_output()?, &fallback)
    })
    .await
    .unwrap_or_else(|_| Err(io::Error::other(crate::i18n::tr(error_label))))
}

mod menu;
pub(super) use menu::Controller as ShellMenuController;

fn helper_command() -> io::Result<Command> {
    let executable = std::env::current_exe()?;
    let mut command = Command::new(executable.with_file_name("strata-shell.exe"));
    crate::background_process::configure(&mut command);
    // The target application must not inherit Strata's private GTK runtime.
    for name in [
        "GSETTINGS_SCHEMA_DIR",
        "GIO_EXTRA_MODULES",
        "GST_PLUGIN_SYSTEM_PATH",
        "GST_PLUGIN_SCANNER",
        "GDK_PIXBUF_MODULE_FILE",
        "GDK_PIXBUF_MODULEDIR",
        "FONTCONFIG_PATH",
        "DBUS_SESSION_BUS_ADDRESS",
        "GDK_DISABLE",
        "GSK_RENDERER",
    ] {
        command.env_remove(name);
    }
    if let Some(path) = std::env::var_os("PATH") {
        let own_bin = executable.parent();
        let paths = std::env::split_paths(&path).filter(|path| {
            !own_bin.is_some_and(|own| {
                own.as_os_str()
                    .as_encoded_bytes()
                    .eq_ignore_ascii_case(path.as_os_str().as_encoded_bytes())
            })
        });
        if let Ok(path) = std::env::join_paths(paths) {
            command.env("PATH", path);
        }
    }
    Ok(command)
}

fn helper_result(output: Output, fallback: &str) -> io::Result<()> {
    if output.status.success() {
        return Ok(());
    }
    tracing::warn!(status = ?output.status, "Windows shell helper failed");
    let detail = String::from_utf8_lossy(&output.stderr);
    let detail = detail.trim();
    Err(io::Error::other(
        if detail.is_empty() { fallback } else { detail }.to_owned(),
    ))
}

pub(super) fn choose(paths: Vec<PathBuf>, parent: &impl IsA<gtk::Widget>) {
    let parent = parent.as_ref().downgrade();
    glib::spawn_future_local(async move {
        for path in paths {
            if parent
                .upgrade()
                .is_none_or(|parent| parent.root().is_none())
            {
                break;
            }
            let result = launch(path, true).await;
            if let Some(parent) = parent.upgrade() {
                report(&parent, result);
            }
        }
    });
}

pub(super) fn open(path: PathBuf, parent: &impl IsA<gtk::Widget>) {
    let parent = parent.as_ref().downgrade();
    glib::spawn_future_local(async move {
        let result = launch(path, false).await;
        if let Some(parent) = parent.upgrade() {
            report(&parent, result);
        }
    });
}

pub(super) fn properties(paths: Vec<PathBuf>, parent: &impl IsA<gtk::Widget>) {
    #[cfg(test)]
    if PROPERTIES_CAPTURE.with_borrow_mut(|capture| {
        if let Some(requests) = capture {
            requests.push(paths.clone());
            true
        } else {
            false
        }
    }) {
        return;
    }
    let parent = parent.as_ref().downgrade();
    glib::spawn_future_local(async move {
        let result = run_helper(paths, "--properties", "Unable to open Windows properties").await;
        if let Err(error) = result {
            tracing::warn!(%error, "Windows properties helper failed");
            if let Some(parent) = parent.upgrade() {
                super::modal::show_error_dialog(
                    &parent,
                    &crate::i18n::tr("Unable to open Windows properties"),
                    &crate::services::io_error_detail(&error),
                );
            }
        }
    });
}

#[cfg(test)]
thread_local! {
    static PROPERTIES_CAPTURE: std::cell::RefCell<Option<Vec<Vec<PathBuf>>>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(in crate::ui) fn capture_properties_for_test(
    capture: Option<Vec<Vec<PathBuf>>>,
) -> Option<Vec<Vec<PathBuf>>> {
    PROPERTIES_CAPTURE.replace(capture)
}

pub(super) fn report(parent: &impl IsA<gtk::Widget>, result: io::Result<()>) {
    if let Err(error) = result {
        tracing::warn!(%error, "unable to open Windows file");
        super::modal::show_error_dialog(
            parent,
            &crate::i18n::tr("Unable to open file"),
            &crate::services::io_error_detail(&error),
        );
    }
}

#[cfg(test)]
mod tests;
