// SPDX-License-Identifier: MIT

use gtk::prelude::*;

#[test]
fn bundled_font_shapes_vietnamese_without_a_display() {
    crate::assets::prepare().expect("register bundled fonts");
    super::configure_font_map();
    let surface =
        cairo::ImageSurface::create(cairo::Format::ARgb32, 320, 64).expect("image surface");
    let context = cairo::Context::new(&surface).expect("drawing context");
    let layout = pangocairo::functions::create_layout(&context);
    layout.set_font_description(Some(&gtk::pango::FontDescription::from_string(
        "JetBrains Mono 12",
    )));
    layout.set_text("Strata — Tiếng Việt: thư mục, tệp ảnh 0123456789");
    assert_eq!(layout.unknown_glyphs_count(), 0);
    pangocairo::functions::show_layout(&context, &layout);
    context.status().expect("render bundled font");
}

#[test]
fn fontconfig_preserves_windows_script_fallback() {
    let native = pangocairo::FontMap::for_font_type(cairo::FontType::FontTypeWin32)
        .expect("Windows font backend");
    let fontconfig = pangocairo::FontMap::for_font_type(cairo::FontType::FontTypeFt)
        .expect("Fontconfig font backend");
    let native = gtk::pango::Layout::new(&native.create_context());
    let fontconfig = gtk::pango::Layout::new(&fontconfig.create_context());
    for text in [
        "Tiếng Việt — français, español, português, Deutsch, italiano",
        "Русский",
        "日本語 简体中文 한국어",
        "🙂 📁",
    ] {
        native.set_text(text);
        fontconfig.set_text(text);
        assert!(
            fontconfig.unknown_glyphs_count() <= native.unknown_glyphs_count(),
            "Fontconfig lost system font coverage for {text:?}"
        );
    }
}

#[test]
fn direct_composition_detection_matches_gdk_debug_tokens() {
    use std::ffi::OsStr;

    assert!(!super::direct_composition_enabled(None));
    assert!(!super::direct_composition_enabled(Some(OsStr::new(""))));
    assert!(!super::direct_composition_enabled(Some(OsStr::new(
        "opengl,vulkan,d3d12,misc"
    ))));
    assert!(super::direct_composition_enabled(Some(OsStr::new("dcomp"))));
    assert!(super::direct_composition_enabled(Some(OsStr::new(
        "opengl:DComp;misc"
    ))));
    assert!(super::direct_composition_enabled(Some(OsStr::new("all"))));
}

#[test]
fn runtime_environment_populates_glib_visible_win32_defaults() {
    super::configure_runtime_environment();
    assert!(gtk::glib::getenv("DBUS_SESSION_BUS_ADDRESS").is_some_and(|value| !value.is_empty()));
    if !super::direct_composition_enabled(std::env::var_os("GDK_DEBUG").as_deref()) {
        assert!(
            gtk::glib::getenv("GDK_DISABLE").is_some_and(|value| !value.is_empty()),
            "GLib must observe GDK_DISABLE set during startup"
        );
        assert!(
            gtk::glib::getenv("GSK_RENDERER").is_some_and(|value| !value.is_empty()),
            "GLib must observe GSK_RENDERER set during startup"
        );
    }
}
