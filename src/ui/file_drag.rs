// SPDX-License-Identifier: MIT

use std::{cell::Cell, rc::Rc, sync::{OnceLock, atomic::{AtomicU64, Ordering}, mpsc}, time::Instant};
use gtk::prelude::*;

static NEXT: AtomicU64 = AtomicU64::new(1);
static TRACE: OnceLock<Option<mpsc::SyncSender<String>>> = OnceLock::new();

fn trace(stage: &str, source: &gtk::DragSource, session: u64, origin: &str, detail: &str) {
    let writer = TRACE.get_or_init(|| {
        if std::env::var_os("STRATA_DND_DIAGNOSTICS").as_deref() != Some(std::ffi::OsStr::new("1")) { return None; }
        let (sender, receiver) = mpsc::sync_channel::<String>(128);
        std::thread::Builder::new().name("strata-dnd-trace".into()).spawn(move || {
            for event in receiver { tracing::info!(event, "file drag diagnostics"); }
        }).ok()?;
        Some(sender)
    });
    let Some(writer) = writer else { return; };
    static STARTED: OnceLock<Instant> = OnceLock::new();
    let elapsed = STARTED.get_or_init(Instant::now).elapsed().as_micros();
    #[cfg(windows)]
    let native = {
        let handle = source.widget().and_then(|widget| widget.root()).and_then(|root| root.downcast::<gtk::ApplicationWindow>().ok())
            .and_then(|window| super::windows_window_state::handle(&window)).unwrap_or_default();
        format!("{:?}", strata_win_shell::window::drag_state(handle))
    };
    #[cfg(not(windows))]
    let native = "non_windows";
    let _ = writer.try_send(format!("t_us={elapsed} session={session} origin={origin} stage={stage} actions={:?} native={native} {detail}", source.actions()));
}

/// Observe every file source without taking focus or altering GTK's cancel policy.
/// Install before the source's prepare callback so the signal accumulator sees us.
pub(super) fn install(source: &gtk::DragSource, origin: &'static str) {
    let session = Rc::new(Cell::new(0));
    let prepared = session.clone();
    source.connect_prepare(move |source, _, _| {
        prepared.set(NEXT.fetch_add(1, Ordering::Relaxed));
        trace("prepare", source, prepared.get(), origin, "");
        None
    });
    let begun = session.clone();
    source.connect_drag_begin(move |source, drag| {
        if begun.get() == 0 { begun.set(NEXT.fetch_add(1, Ordering::Relaxed)); }
        trace("begin", source, begun.get(), origin, &format!("formats={:?}", drag.formats()));
    });
    let canceled = session.clone();
    source.connect_drag_cancel(move |source, _, reason| {
        trace("cancel", source, canceled.get(), origin, &format!("reason={reason:?}"));
        false
    });
    source.connect_drag_end(move |source, drag, delete_data| {
        super::browser::finish_file_drag(&drag.content());
        trace("end", source, session.replace(0), origin, &format!("effect={:?} delete_requested={delete_data}", drag.selected_action()));
    });
}
