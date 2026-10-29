// SPDX-License-Identifier: MIT

use gtk::prelude::*;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
    sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    time::Instant,
};

static NEXT: AtomicU64 = AtomicU64::new(1);
static DROPPED: AtomicU64 = AtomicU64::new(0);
static SEQUENCE: AtomicU64 = AtomicU64::new(1);
static TRACE: OnceLock<Option<mpsc::SyncSender<String>>> = OnceLock::new();

fn trace(stage: &str, source: &gtk::DragSource, session: u64, origin: &str, detail: &str) {
    let writer = TRACE.get_or_init(|| {
        if std::env::var_os("STRATA_DND_DIAGNOSTICS").as_deref() != Some(std::ffi::OsStr::new("1"))
        {
            return None;
        }
        let (sender, receiver) = mpsc::sync_channel::<String>(128);
        std::thread::Builder::new()
            .name("strata-dnd-trace".into())
            .spawn(move || {
                for event in receiver {
                    tracing::info!(event, "file drag diagnostics");
                }
            })
            .ok()?;
        Some(sender)
    });
    let Some(writer) = writer else {
        return;
    };
    static STARTED: OnceLock<Instant> = OnceLock::new();
    let elapsed = STARTED.get_or_init(Instant::now).elapsed().as_micros();
    #[cfg(windows)]
    let native = {
        let handle = source
            .widget()
            .and_then(|widget| widget.root())
            .and_then(|root| root.downcast::<gtk::ApplicationWindow>().ok())
            .and_then(|window| super::windows_window_state::handle(&window))
            .unwrap_or_default();
        format!("{:?}", strata_win_shell::window::drag_state(handle))
    };
    #[cfg(not(windows))]
    let native = "non_windows";
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let dropped = DROPPED.swap(0, Ordering::Relaxed);
    if writer.try_send(format!("t_us={elapsed} seq={sequence} dropped={dropped} session={session} origin={origin} stage={stage} actions={:?} native={native} {detail}", source.actions())).is_err() {
        DROPPED.fetch_add(dropped.saturating_add(1), Ordering::Relaxed);
    }
}

/// Call the returned observer from the actual prepare handler: GTK's prepare
/// signal uses a first-wins accumulator, so an extra handler would replace it.
#[must_use = "Call this observer from the source's provider callback"]
pub(super) fn install(
    source: &gtk::DragSource,
    origin: &'static str,
) -> impl Fn(&gtk::DragSource) + use<> {
    let session = Rc::new(Cell::new(0));
    let active = Rc::new(RefCell::new(HashMap::<usize, u64>::new()));
    let begun = session.clone();
    let begun_active = active.clone();
    source.connect_drag_begin(move |source, drag| {
        if begun.get() == 0 {
            begun.set(NEXT.fetch_add(1, Ordering::Relaxed));
        }
        begun_active
            .borrow_mut()
            .insert(drag.as_ptr() as usize, begun.get());
        trace(
            "begin",
            source,
            begun.get(),
            origin,
            &format!("formats={:?}", drag.formats()),
        );
    });
    let canceled = active.clone();
    source.connect_drag_cancel(move |source, drag, reason| {
        let id = canceled
            .borrow()
            .get(&(drag.as_ptr() as usize))
            .copied()
            .unwrap_or_default();
        trace("cancel", source, id, origin, &format!("reason={reason:?}"));
        false
    });
    let ended = session.clone();
    source.connect_drag_end(move |source, drag, delete_data| {
        super::browser::finish_file_drag(&drag.content());
        let id = active
            .borrow_mut()
            .remove(&(drag.as_ptr() as usize))
            .unwrap_or_default();
        if ended.get() == id {
            ended.set(0);
        }
        trace(
            "end",
            source,
            id,
            origin,
            &format!(
                "effect={:?} delete_requested={delete_data}",
                drag.selected_action()
            ),
        );
    });
    move |source| {
        session.set(NEXT.fetch_add(1, Ordering::Relaxed));
        trace("prepare", source, session.get(), origin, "");
    }
}

#[cfg(test)]
mod tests;
