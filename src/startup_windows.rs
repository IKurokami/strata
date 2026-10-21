// SPDX-License-Identifier: MIT

use std::ffi::OsStr;

use gtk::prelude::*;

const DIRECT_COMPOSITION_DISABLED_BACKENDS: &str = "d3d11,d3d12,gl,vulkan";

/// Configures Win32 GTK/GIO environment defaults before GTK or GIO initializes
/// its display and session-bus singletons.
///
/// In GTK 4.24 on Windows, DirectComposition (`dcomp_device`) is disabled
/// unless `GDK_DEBUG=dcomp` is explicitly set. Without DirectComposition,
/// `gdk_win32_gl_context_surface_attach` and
/// `gdk_win32_vulkan_context_surface_attach` unconditionally reject every
/// surface after `gdk_win32_display_open` and `gsk_renderer_new_for_surface`
/// have already created and torn down D3D11, D3D12, three WGL OpenGL contexts,
/// and a Vulkan device (~800–3,000 ms on hybrid/discrete GPUs) before falling
/// back to Cairo. Skip those unused GPU probes unless the user opted into
/// DirectComposition or explicitly chose a renderer or GDK disable mask.
pub(crate) fn configure_runtime_environment() {
    configure_bundle_paths();
    set_env_if_unset("DBUS_SESSION_BUS_ADDRESS", "disabled:");
    let gdk_debug = std::env::var_os("GDK_DEBUG");
    if !direct_composition_enabled(gdk_debug.as_deref()) {
        set_env_if_unset("GDK_DISABLE", DIRECT_COMPOSITION_DISABLED_BACKENDS);
        set_env_if_unset("GSK_RENDERER", "cairo");
    }
}

fn configure_bundle_paths() {
    let Ok(executable) = std::env::current_exe() else {
        return;
    };
    let Some(root) = executable.parent().and_then(std::path::Path::parent) else {
        return;
    };
    if !root
        .join("share/glib-2.0/schemas/gschemas.compiled")
        .is_file()
    {
        return;
    }
    for (name, relative) in [
        ("GSETTINGS_SCHEMA_DIR", "share/glib-2.0/schemas"),
        ("GIO_EXTRA_MODULES", "lib/gio/modules"),
        ("GST_PLUGIN_SYSTEM_PATH", "lib/gstreamer-1.0"),
        (
            "GST_PLUGIN_SCANNER",
            "libexec/gstreamer-1.0/gst-plugin-scanner.exe",
        ),
        (
            "GDK_PIXBUF_MODULE_FILE",
            "lib/gdk-pixbuf-2.0/2.10.0/loaders.cache",
        ),
        ("GDK_PIXBUF_MODULEDIR", "lib/gdk-pixbuf-2.0/2.10.0/loaders"),
        ("FONTCONFIG_PATH", "etc/fonts"),
        ("STRATA_FFMPEG", "bin/ffmpeg.exe"),
        ("STRATA_FFPROBE", "bin/ffprobe.exe"),
    ] {
        set_env_if_unset(name, root.join(relative).as_os_str());
    }
}

pub(crate) fn direct_composition_enabled(gdk_debug: Option<&OsStr>) -> bool {
    let Some(gdk_debug) = gdk_debug else {
        return false;
    };
    gdk_debug
        .to_string_lossy()
        .split([':', ';', ',', ' ', '\t'])
        .any(|token| token.eq_ignore_ascii_case("dcomp") || token.eq_ignore_ascii_case("all"))
}

#[expect(
    unsafe_code,
    reason = "startup sets process-wide Win32 environment defaults before GTK or worker threads start"
)]
fn set_env_if_unset(key: &str, value: impl AsRef<OsStr>) {
    if std::env::var_os(key).is_some_and(|existing| !existing.is_empty()) {
        return;
    }
    // SAFETY: Called during single-threaded process startup before GTK, GIO, or
    // application worker threads are spawned.
    unsafe {
        std::env::set_var(key, value);
    }
}

pub(crate) fn configure_font_map() {
    if std::env::var_os("PANGOCAIRO_BACKEND").is_some() {
        return;
    }
    // Reuse the Fontconfig font set populated by assets::prepare instead of
    // separately enumerating every Windows font through Pango's Win32 backend.
    if let Some(map) = pangocairo::FontMap::for_font_type(cairo::FontType::FontTypeFt)
        .and_then(|map| map.dynamic_cast::<pangocairo::FontMap>().ok())
    {
        pangocairo::FontMap::set_default(Some(&map));
    }
}

#[cfg(test)]
mod tests;
