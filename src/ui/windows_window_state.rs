// SPDX-License-Identifier: MIT

use std::{cell::Cell, ffi::c_void};

use gtk::{glib, prelude::*};
use serde::{Deserialize, Serialize};
use strata_win_shell::window::{self, Placement};

#[derive(Deserialize, Serialize)]
struct SavedPlacement {
    normal: [i32; 4],
    maximized: bool,
}

fn path() -> std::path::PathBuf {
    crate::storage::config_directory().join("window.toml")
}

#[expect(
    unsafe_code,
    reason = "GDK exposes native HWND access only through its Win32 C API"
)]
pub(super) fn handle(window: &gtk::ApplicationWindow) -> Option<isize> {
    use glib::translate::ToGlibPtr;
    unsafe extern "C" {
        fn gdk_win32_surface_get_handle(surface: *mut gtk::gdk::ffi::GdkSurface) -> *mut c_void;
    }
    let surface = window.surface()?;
    // SAFETY: This Windows-only browser owns a live Win32 GdkSurface. The
    // borrowed surface remains alive for the entire native-handle lookup.
    let handle = unsafe { gdk_win32_surface_get_handle(surface.to_glib_none().0) };
    (!handle.is_null()).then_some(handle as isize)
}

fn save(window: &gtk::ApplicationWindow) {
    let Some(handle) = handle(window) else { return };
    let Ok(placement) = window::placement(handle) else {
        return;
    };
    let saved = SavedPlacement {
        normal: placement.normal,
        maximized: placement.maximized,
    };
    let result = (|| {
        let path = path();
        std::fs::create_dir_all(path.parent().expect("window state directory"))?;
        let contents = toml::to_string(&saved).map_err(std::io::Error::other)?;
        crate::storage::atomic_write(&path, contents.as_bytes())
    })();
    if let Err(error) = result {
        tracing::warn!(%error, "unable to save browser window placement");
    }
}

pub(super) fn bind(window: &gtk::ApplicationWindow) {
    let saved = std::fs::read_to_string(path())
        .ok()
        .and_then(|contents| toml::from_str::<SavedPlacement>(&contents).ok());
    let restored = Cell::new(false);
    window.connect_map(move |window| {
        if restored.replace(true) {
            return;
        }
        if let Some(saved) = saved.as_ref()
            && let Some(handle) = handle(window)
            && let Err(error) = window::restore(
                handle,
                Placement {
                    normal: saved.normal,
                    maximized: saved.maximized,
                },
            )
        {
            tracing::warn!(%error, "unable to restore browser window placement");
        }
    });
    window.connect_close_request(|window| {
        save(window);
        glib::Propagation::Proceed
    });
    window.connect_unmap(save);
}
