# Developing and running Strata on Windows

Use MSYS2 **UCRT64** for the GTK application. The normal Rustup MSVC toolchain
can test the independent native I/O crate when Visual C++ Build Tools and the
Windows SDK are installed; it cannot link the UCRT64 GTK libraries.

## Install the native build environment

Install MSYS2 from its official WinGet package:

```powershell
winget install --id MSYS2.MSYS2 --exact --source winget
```

In the MSYS2 UCRT64 terminal, update the distribution (restart the terminal if
the core update requests it), then install the same packages as the Windows CI:

```bash
pacman -Syu
pacman -S --needed mingw-w64-ucrt-x86_64-{rust,gcc,clang,pkgconf,gtk4,gtksourceview5,gstreamer,ffmpeg,gst-plugins-base,gst-plugins-good,gst-libav,poppler,glib2,fontconfig,cmake,meson,ninja,shaderc,vulkan-headers}
```

## Build, test, run, and package

From a PowerShell terminal in the repository:

```powershell
./scripts/windows/dev.ps1 build
./scripts/windows/dev.ps1 test
./scripts/windows/dev.ps1 run
./scripts/windows/dev.ps1 package
```

For a non-default installation, add `-UcrtRoot 'D:\msys64\ucrt64'`. To open a
specific folder, use `run -ApplicationArguments 'C:\Users\example\Documents'`.
The script sets toolchain paths only for its process and restores them afterward;
it does not change the machine's PATH or Rustup's default toolchain.

The test action runs native I/O tests and the Windows safety and filesystem
regressions in `scripts/windows/regression-tests.txt`, requiring each regression
to exist. The linked-executable CI workflow uses the same inventory, including
rename, move, batch copy and cancellation queue draining. GTK interaction tests still
need a separately isolated display/session; this command does not run them on
the user's desktop. A successful `--version` alone does not prove first paint.

The package action creates a fresh timestamped self-contained portable release
directory under `target/` and writes `release-manifest.json` with SHA-256 hashes.
Copy or extract the complete directory, then launch `Start-Strata.exe`.
This native launcher and the application use the Windows GUI subsystem, so
opening them from Explorer creates no CMD window. `Start-Strata.cmd` remains
available for terminal diagnostics: it explicitly sets
`STRATA_ATTACH_PARENT_CONSOLE=1` to attach to its caller's terminal while preserving
redirected streams. Ordinary GUI launches do not attach to a parent console.
Settings → General → Desktop integration →
**Show application console** enables or hides Strata's own diagnostic console
immediately and remembers the choice. It never hides an existing caller terminal.
An allocated console shows the latest 256 KiB of application logs, including
startup diagnostics, then streams new logs. Launcher output redirection remains
active independently of this view.
The tray icon provides Open Strata, the same console toggle and Quit Strata.
Enable **Keep Strata running in the background** to reopen from the tray after
closing the window; Quit retains the active-operation guards.
External opens share one browser window per user, logon session and configuration,
including launches from different installation paths. A matching folder tab is
selected or a new tab is created, and the existing window is restored and activated.
Shell selection registration stays available while the resident window is hidden.
An unavailable activation endpoint fails the launch instead of creating another
browser window.
The executable depends on the adjacent `bin`, `lib`, `libexec`, `share`, and
`etc` directories. The package smoke checks exclude MSYS2 from PATH.

Windows startup reuses the Fontconfig font backend that holds the bundled
JetBrains Mono font, avoiding a second font enumeration through Pango's Win32
backend. An explicitly supplied `PANGOCAIRO_BACKEND` remains authoritative; if
the Fontconfig backend is unavailable, Pango keeps its platform default.
Release bundles include the [private Windows IME runtime](../packaging/windows/runtime/README.md).
Build it before packaging; it fixes input-context teardown and uses native
Windows fonts for IMM32 composition without changing widget font rendering.
The portable launcher and development script also avoid D-Bus autolaunch when
no `DBUS_SESSION_BUS_ADDRESS` is supplied. An explicit session bus is preserved.

## Build a Windows installer

Create a portable bundle from the optimized release binary first. The installer
wraps that complete bundle without rebuilding Rust or fetching runtime packages.
Install Inno Setup 6, then run from PowerShell (including Windows PowerShell 5.1):

```powershell
winget install --id JRSoftware.InnoSetup --exact
./scripts/windows/package-installer.ps1 `
  -Bundle './target/Strata-Windows-Release-20261018-141626' `
  -Destination './target/installer'
```

Replace `-Bundle` with your portable directory containing `release-manifest.json`.
The script verifies its checksums before compiling. For a custom compiler location,
pass `-Compiler 'D:\Inno Setup 6\ISCC.exe'`.

The resulting `Strata-VERSION-windows-x64-setup.exe` installs per user under
`%LOCALAPPDATA%\Programs\Strata`, with a Start Menu shortcut and an optional
Desktop shortcut. Shortcuts launch `bin/strata.exe` directly without CMD.
It requires no administrator rights and does not change file associations until
**Replace File Explorer** is enabled in Settings. It does not change the system
PATH. Uninstall restores this installation's owned folder handlers, then removes application files and shortcuts while
retaining preferences and user data outside the installation directory.
Locally generated installers are unsigned; release signing remains a separate step.

## Current limits

### Windows release updates

Windows checks releases and release notes from `IKurokami/strata`; Linux keeps
the upstream `lgse/strata` feed. Windows update caches have separate filenames so
older Linux-feed responses cannot hide a fork release. An installable Windows
asset is named `strata-VERSION-x86_64-pc-windows-gnu.zip`. Package the complete
portable directory, including its private runtime, into that archive. The archive
may contain the bundle directly or one enclosing directory; updates replace the
bundle root, including `bin/`, rather than copying another `bin/` into it.

Publish `strata-update-manifest.json` and
`strata-update-manifest.signatures.json` beside the ZIP. The manifest must identify
`IKurokami/strata`, the exact tag, and the `x86_64-pc-windows-gnu` artifact.
Generate and sign it with:

```powershell
python scripts/sign_update_manifest.py --platform windows --directory dist `
  --version VERSION --source-commit SOURCE_SHA
```

As with Linux signing, `UPDATE_SIGNING_KEY` supplies the private Ed25519 PEM.
Windows trusts only public keys compiled from `data/windows-update-keys.json`;
upstream's Linux signing key does not authorize fork updates. The Windows trust
store is initially empty until the fork owner provisions their release key.
Unsigned releases and releases without a trusted Windows signature cannot be
installed in the app. Keep the private key outside the checkout and release
archives. See [signed updates](signed-updates.md) for the signature format.

Windows video seeks reuse successful file probes while the file's metadata
fingerprint and decoder tool are unchanged. Replacing the file invalidates those
results. Decoder cancellation and process reaping run outside GTK's event loop.

Windows builds embed `data/brand/strata.ico` in both executables. The installer
shortcuts and the running application share `io.github.lgse.Strata` as their
AppUserModelID, so the taskbar uses the same identity as the installed shortcut.
The GNU build needs `windres` on PATH (`WINDRES` can select a cross compiler).
After changing the application SVG, regenerate its Windows icon with
`python scripts/windows/generate-icon.py --renderer rsvg-convert`.

Ordinary local file activation uses the native Windows Shell in `strata-shell.exe`, which
honors Windows default applications, shortcuts, packaged applications and UAC
requests from executable manifests. An unknown file type and **Open With** use
the Windows application chooser. Cancelling a chooser or UAC prompt is harmless.
The shell helper runs separately from GTK so a failing shell extension cannot
crash the file-manager process. Keep `strata-shell.exe` beside `strata.exe`,
including for development builds (`cargo build --bins`).
Explicit video start times retain the existing player-specific launch path;
if its handler is unavailable or rejects the launch, Windows opens the file normally.

For faster reopening, enable **Settings → General → Startup → Keep Strata running
in the background**. Close retains one initialized window and clears its media
preview. Open Strata again to reuse it; explicit paths still navigate to the
requested target in a matching or new tab. Opening Strata without a path,
including Win+E and the tray action, brings the existing window forward.
Base styles and the bundled icon resource path are registered once per display,
so repeated activation does not reload icons or restyle existing widgets.
Use **Quit Strata** in
Settings or `Start-Strata.cmd --quit` to exit completely. The option is off by
default and does not register a login task.
Windows activation uses a bounded local named pipe with a current-user-only ACL,
session/configuration isolation, and no remote clients. Native directory scans
start off the GTK thread while eligible startup windows are being mapped. Starts
on fixed local drives without a saved session can overlap mapping. Saved sessions
are read and validated on a worker while the interface is constructed, then
published on GTK's main thread. A late read cannot replace subsequent navigation
or reopen a closed window. Removable drives and remote startup targets retain
deferred navigation. Hidden RAW metadata rows are created only when needed.

Build both Windows binaries before packaging:

```powershell
cargo build --release --locked --no-default-features --bins
```

`strata-shell.exe` is a standalone
proxy used by **Settings → General → Desktop integration → Replace File Explorer**.
The default folder/drive verbs and their rollback journal are per-user; enabling
requires no administrator rights. Turning it off or uninstalling restores previous
defaults without overwriting another file manager's subsequent changes. After
manual deletion of a portable directory, the retained helper repairs the defaults
within its 500 ms target check, or on the next folder open if it was not running.
Its dedicated native keyboard thread routes Win+E to Strata while enabled; the
helper starts at sign-in through a per-user Run entry. Disabling or uninstalling
stops the helper and restores the prior entry.
Win+E sends a bounded activation request directly to an existing Strata process,
avoiding another GTK executable load. If its endpoint is absent, the helper
starts the application normally.

Windows uses one Strata browser window. Plain launches and Win+E present that
window; explicit folder requests activate an existing matching tab or open a new
tab, preserving the other tabs. The legacy new-window action also routes to a
tab on Windows.
The visible native-folder tab registers with Windows Shell so callers such as
Chrome's **Show in folder** can select the requested file through
`SHOpenFolderAndSelectItems`. Navigation, closing the window, and disabling
Explorer replacement revoke that registration. Shell requests cross to GTK on
the main thread; COM calls run in their own apartment.
The uninstaller also requests
normal application shutdown, including an idle retained window, before removing
files. Windows file dialogs retain their
system behavior. See [preference lifecycle](preferences.md) for saved-window behavior.

On Windows, the Trash sidebar opens the current user's native Recycle Bin inside
Strata, across local volumes. Original filenames and extensions do not depend on
Explorer's extension visibility setting. Restore and permanent deletion use each
deleted item's Shell identity, so repeated deletions from the same path remain
distinct. Folder contents can be browsed without allowing mutations inside a
deleted folder. Empty Trash measures the native items and uses Shell deletion.

The native paths for navigation, rename/move, batch paste, Recycle Bin/undo,
archive operations, rich previews, custom actions, external providers and
verified updates are implemented in this branch. They still require the
isolated Windows GUI smoke, fault-injection matrix and clean-machine evidence
before being called release-verified. See [feature compatibility](windows-feature-compatibility.md)
and [port milestones](windows-port.md).
