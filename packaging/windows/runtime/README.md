# Windows GTK and playback runtime

The GTK patch also maps the explicit outgoing MIME type
`application/x.windows.CF_HDROP` to the predefined Windows `CF_HDROP` value.
GTK 4.24.1 otherwise registers the string `CF_HDROP` as a new format, which
native file drop targets do not recognize. Strata supplies a `DROPFILES` header
and a double-NUL-terminated UTF-16 path list alongside its GTK FileList and URI
providers. List, icon, column, search and preview drags share this provider.
Mixed non-native selections retain the GTK/URI formats without exporting a
partial native file selection. The GTK and GstPlay lifetime fixes below remain
required and are preserved; no dependency baseline or source hash changed.

The same patch must classify `CF_HDROP` as HGLOBAL-backed data. GTK 4.24.1 and
upstream main inspected on 2026-10-24 omit it from
`_gdk_win32_format_uses_hdata`. Mapping the format without this classification
routes a DROPFILES payload through the single-handle stream and writes through
a null pointer. Two local crash dumps and the actual private stream reproduce
the access violation. Adding `CF_HDROP` to the HGLOBAL cases preserves ownership:
the stream allocates movable global memory, writes the complete payload and
hands the handle to the OLE consumer, which releases the returned STGMEDIUM.
The IME, font backend and GstPlay patches remain required and unchanged.

`scripts/windows/test-file-drop-stream.py RUNTIME_DIRECTORY` tests the exact
loaded DLL without initializing GTK, opening a window or touching the clipboard.
It resolves private entry points from the pinned build's retained COFF symbols;
missing symbols fail the probe. Both synchronous and asynchronous chunked writes
must produce an HGLOBAL that `DragQueryFileW` reads as the complete Unicode and
UNC file selection. `--expect-unsupported` checks the faulty baseline's
classification without deliberately crashing it. Running the normal probe on
the previous mapping-only runtime reproduces exit code `-1073741819`.
The runtime lifecycle runner includes this probe. This verifies serialization
and native ownership, not a physical drag into a particular third-party app.
Use `test-gtk-runtime.ps1 -ExpectFileDropUnsupported` only for the mapping-only
baseline; candidate and release checks must use the normal, required stream test.

`file-drop.c` checks this mapping through the native OLE `IDataObject` consumer
and `DragQueryFileW`, including multiple Unicode and UNC paths. It creates a
private window station and desktop with their own clipboard and stops if that
isolation is unavailable; it never replaces the user's clipboard. Some Windows
accounts cannot create a fresh station (errors 5/183); this is a blocked probe,
not a passing or silently skipped native interoperability check. Compile with
`gcc -Wall -Wextra -Werror file-drop.c -o file-drop.exe $(pkgconf --cflags --libs gtk4) -lole32 -lshell32 -luser32`.
Place the executable beside the candidate GTK DLL and verify the printed loaded
path. The candidate must pass normally. An otherwise identical runtime without
the file-format hunk must pass `--expect-unsupported`, demonstrating that it
offers a registered name instead of the predefined format. This is a native
data-format contract check, not proof of a physical mouse drag into every app;
test those interactions on the installed artifact before promotion.

GTK 4.24.1 registers a Win32 display message filter with a borrowed
`GtkIMContextIME` pointer. Losing the client surface before focus-out, disposing
the context, or repeated focus-in can leave the filter referencing freed memory.
An IME signal handler can also release the context during message dispatch.
The patch tracks the filter's display independently of the widget, removes it
on unrealize/client replacement/disposal, prevents duplicate registration, and
holds the context while emitting callbacks.

The widget may use PangoCairo's Fontconfig backend. IMM32 composition fonts
still require a Win32 `LOGFONT`; the patch uses a separate native Pango context
for that lookup. It preserves Fontconfig widget rendering and Windows IME.
The native font map is created lazily and reused per display, avoiding repeated
Windows font enumeration on subsequent IME notifications.

The relevant ownership paths remain unfixed in upstream GTK 4.24.1 and the
upstream main source inspected on 2026-10-22. This is a private, version-specific
patch, not an upstream-approved fix. Before updating GTK, compare these paths
and rebase or retire the patch with regression evidence.

The build also includes GstPlay 1.28.8 with a version-specific rebase of the
worker-disposal candidate in [`../../media-runtime/README.md`](../../media-runtime/README.md).
The unpatched 1.28.8 baseline can release its last reference on the playback
worker before that worker finishes using the object. The patch retains it
until worker teardown completes, including repeated disposal; disposal on
another thread retains its join behavior. This is a bounded ownership fix,
not a full concurrency audit or a claim that all media RAM retention is fixed.

Source: <https://download.gnome.org/sources/gtk/4.24/gtk-4.24.1.tar.xz>

SHA-256: `e98abe720e16129c8c0f50761dee0a2e9ae2478055e31018f6cf977fcf6513e9`.
This matches the MSYS2 GTK 4.24.1 package recipe. The build retains MSYS2's font
hinting patch, pinned to
`ebac78616a7668edbfdd77b9959ad63239aa59ed12b995a9663bf2df7158be26`.
GstPlay source: <https://gstreamer.freedesktop.org/src/gst-plugins-bad/gst-plugins-bad-1.28.8.tar.xz>,
SHA-256 `670d888bde8242579b0d8e91f60cf337aa7be20ca7c315ebfbdd6e5a9218d104`.
Only the GstPlay DLL is rebuilt; unrelated MSYS2 plugin patches and plugin
libraries are not rebuilt or replaced. Matching GStreamer 1.28.8 dependencies
and decoder plugins remain necessary.
No system DLL is replaced. GTK's license is LGPL-2.1-or-later; the modified C
file retains its LGPL-2.0-or-later notice and original authors. GstPlay retains
its LGPL-2.0-or-later source notice and authors listed in the original kit.
Corresponding source archives, patches, licenses, and runtime hashes accompany the portable bundle
under `share/strata/runtime/`. The original probe is MIT-licensed.

Use native UCRT64 GCC, pkgconf, Ninja, Meson, GTK/GStreamer development
dependencies, shaderc, Vulkan headers and loader. Python-hosted Meson must use
native Windows Python. Build a fresh private runtime before release packaging:

```powershell
./scripts/windows/build-gtk-runtime.ps1 `
  -BuildRoot './target/gtk-runtime-build' `
  -Destination './target/windows-gtk-runtime'
```

`package-release.ps1` requires this runtime and verifies its source/patch/DLL
identity. `-GtkRuntime` selects another verified runtime directory. Dependencies
are inherited from the UCRT64 runner; source pinning alone does not pin the
entire compiler/dependency graph. Vulkan, OpenGL and GStreamer support remain
enabled. This recipe currently targets Windows x64 only.

Compile `ime-lifetime.c` with `pkgconf --cflags --libs gtk4 pangocairo` and
`-limm32`. Run each mode in a separate process with `G_DEBUG=fatal-criticals`:
`dispose`, `detach`, `unrealize`, `repeat-focus`, `reentrant`, `native-font`,
and `fontconfig`. Use an otherwise identical unpatched build for comparison.
Place the probe beside the candidate DLL and verify the loaded module path;
changing PATH alone cannot override a DLL beside an executable. The probe sends
messages exclusively to its own window and associates its own IME context.
It does not exercise a user's keyboard or establish physical Telex correctness.

Before promoting an installer, verify the installed private DLL hash and loaded
path, lifecycle probes, normal GTK tab/modal/close behavior, media playback,
and installed upgrade/rollback. Record results and omissions in the handoff;
the existence of this recipe does not mean those checks passed.

The shared GTK lifecycle probe in `packaging/media-runtime/probes/` also runs on
Windows. Its resource column reports process handles there instead of Unix file
descriptors; thread counts come from Toolhelp. Compare 50 playback/close cycles
against the same unpatched GTK baseline with generated media. The GStreamer
worker-lifetime probe remains independent: an IME patch does not fix GstPlay.

`scripts/windows/test-gtk-runtime.ps1` runs all seven IME cases, ten trials for
each of the three GstPlay disposal modes, and fifty GTK playback/close cycles.
The release workflow uses the private DLLs for application GUI tests as well.
