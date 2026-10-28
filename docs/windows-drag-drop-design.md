# Windows outbound file drag lifecycle

This design defines how to diagnose and correct outbound dragging across Windows
applications when foreground changes. It does not establish that a particular
foreground/capture defect has been reproduced. Keep separate evidence for
serialization, GTK/OLE lifetime and physical cross-window input.

## Branch diagnostics

`ui::file_drag` now observes the file sources for Columns, List/Icons, inline
search, preview and Disk Usage. Sidebar and tab reordering remain separate.
Set `STRATA_DND_DIAGNOSTICS=1` before starting a build containing this helper;
enable the `strata::ui::file_drag=info` logging target if the current logging
filter would suppress it. Restart the existing retained process for changed
environment variables to take effect.

Events contain a monotonic timestamp, sequence/session IDs, source category,
action/format information and native origin/foreground/calling-thread capture
state. The bounded logging lane reports dropped records and does not log file
names or contents. Session IDs follow the actual GdkDrag through cancel/end;
cleanup compares provider identity. Provider finalization also retires orphaned
internal associations on the GTK context.

These app-level observations do not instrument the exact native SetCapture call
or prove a successful physical drop. No activation/capture runtime patch is
justified by them alone. The native trace, target harness and installed-artifact
gates below remain necessary before claiming the foreground bug is fixed.

## Existing ownership

Columns (`browser/columns/rows.rs`), List/Icons (`browser_modes.rs`), inline search
(`inline_search/collection.rs`), preview (`preview.rs`) and file-oriented window
controls create drag sources. Tab-strip reordering is a different payload and must
not receive file-drag behavior. Audit the global Smart Search result surface
separately: inline-search coverage does not prove it has a file DragSource.

`browser/clipboard.rs::file_drag_content` supplies GTK FileList and URI providers
plus Windows CF_HDROP for complete native selections. `ACTIVE_DRAG` associates a
weak provider identity with source locations for trusted internal drops. It must
not be confused with the OLE provider's strong lifetime. `browser/transfer.rs`
handles Strata as a receiving application and is not the outbound IDropSource.

The private GTK patch maps the Windows bridge MIME type to predefined CF_HDROP
and classifies its payload as HGLOBAL-backed. Preserve both changes. The existing
stream/native-data probes establish format and memory ownership, not a physical
drag across application windows. See the [Windows runtime contract](../packaging/windows/runtime/README.md).

There is currently no shared cancel trace across all these sources. Inline search
creates a file provider without the same end hook used by Columns and preview.
That is a lifecycle coverage gap to investigate, not proof that it causes the
reported foreground failure.

## Distinguish input scenarios

| Scenario | Evidence required |
| --- | --- |
| Foreground Strata, direct drag to Explorer | Baseline OLE session, native payload and completed target drop |
| Strata initially background, click-and-drag without releasing | Activation ordering relative to the native grab |
| Start dragging, hold mouse and Alt+Tab | Session continuity, grab/capture transitions and cancellation reason |
| Cross monitor or window boundary | DPI-aware coordinates and uninterrupted native target events |
| Explorer into Strata / internal Strata transfer | Independent inbound/internal regression coverage |

Use an actual file-drop target at matching integrity level for the baseline.
Applications that do not accept files, or privilege boundaries that reject a
drop, do not establish a Strata provider failure.

Microsoft documents that background capture is restricted to the visible window
area. However, a click may already activate Strata before GTK grabs input: record
the sequence rather than inferring it from the initial foreground application.
`SetCapture` returns the previous capture HWND, not a success boolean.
`GetCapture` describes the calling thread; measuring it on a worker cannot prove
that the GTK thread lost capture. See [SetCapture](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setcapture)
and [GetCapture](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getcapture).

## Shared diagnostics and session contract

Introduce an opt-in `STRATA_DND_DIAGNOSTICS=1` path. Disabled diagnostics must add
no filesystem probes, hashing, per-motion logging or background polling. Events
use a bounded nonblocking sink, a monotonic timestamp, a process-local session ID
and a sequence number. Include dropped-log counts so gaps cannot be mistaken for
missing native events. Do not log paths, clipboard data or file contents.

Attach one file-drag lifecycle helper at every file source. It observes prepare,
begin, cancel and end without replacing selection or icon behavior. A session
owns its provider association and source identities through completion. Cleanup
is idempotent and scoped by session ID/provider identity: a late end from an old
source must not clear a newly prepared drag. Handle prepared-but-never-started
providers and widget unmap/destruction explicitly.

Keep GTK's cancel return semantics intact: diagnostics must not suppress default
handling. Do not clear provider resources inside a cancel notification if OLE can
still request GetData; mark the terminal reason and finish cleanup at the owning
session's safe end. Do not keep widgets or an entire browser alive solely for
diagnostic records. Never delete source files merely because a GTK callback has
`delete_data=true`; transfer completion/effect must follow existing safe ownership.

Record these transition-only fields:

- stage, session and source surface; offered format names, action mask and count;
- GTK active state, origin/root HWND, current foreground HWND and PID;
- UI-thread ID and same-thread capture HWND; modifiers and terminal reason;
- drag identity and selected effect, with explicit final cleanup state.

Native HWND access belongs in a small Windows-only adapter; common code must
continue building on Linux. Avoid changing unrelated named-pipe activation or
foreground-on-open behavior in `windows_runtime.rs` to fix a drag-specific path.

## Native trace and evidence-based fixes

App signals alone cannot prove ordering inside GDK. On the pinned GTK baseline,
an opt-in runtime trace must correlate `drag_context_grab`, owner/foreground HWND,
SetCapture's previous HWND and same-thread current capture. Also trace
WM_MOUSEACTIVATE, WM_ACTIVATE, WM_ACTIVATEAPP, WM_CAPTURECHANGED, WM_CANCELMODE and
button-up transitions, plus QueryContinueDrag, GiveFeedback, requested data formats
and the DoDragDrop result. Do not trace every mouse move. Confirm `GDK_DEBUG=dnd`
support in the actual packaged build instead of assuming its diagnostics exist.

| Observed failure | Smallest appropriate intervention |
| --- | --- |
| Wrong foreground at the grab after a legitimate click | Correct activation/default WM_MOUSEACTIVATE ordering before the grab |
| Valid OLE session canceled on focus/capture transition | Correct the specific GDK grab/lifecycle decision while preserving real cancellation |
| Native target cannot retrieve CF_HDROP | Check actual loaded runtime, format 15, TYMED_HGLOBAL and provider lifetime |
| Only incompatible/elevated targets reject the drag | Report the capability/privilege limitation; do not force source focus |

Do not force foreground repeatedly, attach input threads, install global mouse
hooks, synthesize input as a production workaround, or ignore all grab-broken
events. In particular, pulling Strata back to the foreground after Alt+Tab defeats
the user's intended target. Escape, target destruction and releasing over an
invalid target must still cancel safely. The source must relinquish capture and
remove the drag icon exactly once after OLE finishes.

If a GTK change is supported by trace, review the pinned source and every existing
IME/font/CF_HDROP/media patch first. Apply a narrow, exact patch; keep corresponding
source, licenses, hashes and build provenance together. Validate both unpatched
and patched baselines, actual loaded DLL paths/hashes, lifecycle probes and the
real package. Do not replace system DLLs or infer release inclusion from a Rust
crate update. See the [media runtime rules](../packaging/media-runtime/README.md).

## Native target and input harness

Build a small, separate native OLE target using RegisterDragDrop/IDropTarget. Its
test protocol emits DragEnter, DragOver, DragLeave and Drop transitions, effects,
HRESULTs and a fixture-scoped CF_HDROP summary. GetData must use format 15,
DVASPECT_CONTENT and TYMED_HGLOBAL, validate DROPFILES with DragQueryFileW and
release STGMEDIUM through the native ownership API. Never use private user files.

The coordinator creates Unicode/multi-file fixtures in its own temporary root.
It compares selected fixture identities and contents after copy/move; CF_HDROP
receipt alone is not proof of a completed transfer. Cancellation checks preserve
source hashes and require no target creation. A receiving target that intentionally
copies files must implement/acknowledge its effect correctly; do not infer MOVE
success from cursor feedback alone.

Exercise real held-button movement between different-process HWNDs. API calls
directly into the transfer service, clipboard population and inbound-only drops
cannot substitute for this route. Cover initial background activation and
Alt+Tab mid-drag separately, plus Escape, invalid destination, repeated cancel,
Unicode/multi-selection, Ctrl/Shift actions, different volumes, mixed DPI,
target closure and source hide/unmap.

Default GUI checks remain isolated. A private Windows desktop/window station must
be established before launching either GUI and must never fall back to the user's
desktop or clipboard. Synthetic input on an inactive private desktop is not proof
of physical input routing. Where an interactive input desktop is required, use a
dedicated authorized runner or an explicitly owner-operated test session; report
unavailable prerequisites as blocked, never as passed or silently skipped.

## Validation and completion criteria

Retain existing payload/stream tests and Linux internal-DnD E2E. Extend behavioral
session tests for repeated terminal callbacks, cancellation followed by a new drag,
source destruction and delayed provider requests. Build the app adapter on both
platforms before introducing a runtime change.

For Windows, compare the same scenarios on the baseline and candidate package,
recording build identity, Windows version, integrity levels, DLL path/hash and
monitor/DPI configuration. A fix is demonstrated only with a failing baseline
timeline, the corresponding corrected ordering/lifetime evidence and a successful
drop into both Explorer and the native target. Keep installed upgrade/rollback
checks separate from development-tree tests.

Stress successful/background/canceled/restarted drags for at least 100 rounds,
tracking handles, GDI objects, private bytes and outstanding sessions. Performance
goals are less than 16 ms added p95 prepare-to-begin latency and no 100 ms UI stalls
introduced by diagnostics/fix work; publish measurements with hardware/build
context. Unit tests do not establish these timing or leak properties.

Store runtime logs/captures outside the source tree, sanitized to session/fixture
metadata. Maintained documentation records the protocol and limitations; PRs
record the observed cause, evidence and unrun gates. A data-format probe passing
without foreground-transition E2E is an incomplete foreground-fix validation.

## References

- [GTK 4.24.1 Win32 drag implementation](https://github.com/GNOME/gtk/blob/4.24.1/gdk/win32/gdkdrag-win32.c).
- [GtkDragSource signals](https://docs.gtk.org/gtk4/class.DragSource.html).
- [WM_MOUSEACTIVATE](https://learn.microsoft.com/en-us/windows/win32/inputdev/wm-mouseactivate),
  [foreground restrictions](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setforegroundwindow)
  and [DoDragDrop](https://learn.microsoft.com/en-us/windows/win32/api/ole2/nf-ole2-dodragdrop).
- [Disk Usage integration](disk-usage-design.md) and [isolated E2E testing](e2e-testing.md).
