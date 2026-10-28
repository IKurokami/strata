// SPDX-License-Identifier: MIT

use super::result::{ContentLocator, FileRevision, SearchHit};
use std::{ffi::OsString, path::Path, process::Command};

pub(crate) fn arguments(
    executable: &Path,
    path: &Path,
    locator: &ContentLocator,
) -> Option<Vec<OsString>> {
    let name = executable.file_stem()?.to_str()?.to_ascii_lowercase();
    let mut arguments = Vec::new();
    match locator {
        ContentLocator::Pdf { page_0, .. } => match name.as_str() {
            "sumatrapdf" => {
                arguments.extend([
                    OsString::from("-page"),
                    page_0.saturating_add(1).to_string().into(),
                    path.as_os_str().to_owned(),
                ]);
            }
            "acrord32" | "acrobat" => {
                arguments.extend([
                    OsString::from("/A"),
                    format!("page={}", page_0.saturating_add(1)).into(),
                    path.as_os_str().to_owned(),
                ]);
            }
            "okular" => {
                arguments.extend([
                    OsString::from("--page"),
                    page_0.saturating_add(1).to_string().into(),
                    path.as_os_str().to_owned(),
                ]);
            }
            "evince" => {
                arguments.extend([
                    OsString::from("--page-index"),
                    page_0.to_string().into(),
                    path.as_os_str().to_owned(),
                ]);
            }
            _ => return None,
        },
        ContentLocator::Code { line_1, .. } => match name.as_str() {
            "code" | "code-insiders" | "codium" => {
                arguments.push("--goto".into());
                let mut target = path.as_os_str().to_owned();
                target.push(format!(":{line_1}"));
                arguments.push(target);
            }
            "zed" => {
                let mut target = path.as_os_str().to_owned();
                target.push(format!(":{line_1}"));
                arguments.push(target);
            }
            _ => return None,
        },
        ContentLocator::Audio { start_us, .. } | ContentLocator::Video { start_us, .. } => {
            match name.as_str() {
                "vlc" => arguments
                    .push(format!("--start-time={:.3}", *start_us as f64 / 1_000_000.0).into()),
                "mpv" => {
                    arguments.push(format!("--start={:.3}", *start_us as f64 / 1_000_000.0).into())
                }
                _ => return None,
            };
            arguments.push(path.as_os_str().to_owned());
        }
        _ => return None,
    }
    Some(arguments)
}

pub(crate) fn open(hit: &SearchHit) -> bool {
    if !std::fs::metadata(&hit.path)
        .is_ok_and(|metadata| FileRevision::from_metadata(&metadata) == hit.revision)
    {
        return false;
    }
    #[cfg(windows)]
    let executable = match strata_win_shell::launch::associated_executable(&hit.path) {
        Ok(path) => path,
        Err(_) => return false,
    };
    #[cfg(not(windows))]
    let executable = {
        use gio::prelude::*;
        let (mime, _) = gio::content_type_guess(Some(&hit.path), None::<&[u8]>);
        let Some(application) = gio::AppInfo::default_for_type(&mime, false) else {
            return false;
        };
        application.executable()
    };
    let Some(arguments) = arguments(&executable, &hit.path, &hit.locator) else {
        return false;
    };
    let mut command = Command::new(executable);
    command.args(arguments);
    crate::background_process::configure(&mut command);
    command.spawn().is_ok()
}

#[cfg(test)]
mod tests;
