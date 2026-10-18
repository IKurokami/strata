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
pacman -S --needed mingw-w64-ucrt-x86_64-{rust,gcc,clang,pkgconf,gtk4,gtksourceview5,gstreamer,ffmpeg,gst-plugins-base,gst-plugins-good,poppler,glib2,fontconfig,cmake}
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
Copy or extract the complete directory, then launch `Start-Strata.cmd`.
The executable depends on the adjacent `bin`, `lib`, `libexec`, `share`, and
`etc` directories. The package smoke checks exclude MSYS2 from PATH.

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
Desktop shortcut. Shortcuts launch the runtime through `Start-Strata.cmd`.
It requires no administrator rights and does not register file associations or
change the system PATH. Uninstall removes application files and shortcuts while
retaining preferences and user data outside the installation directory.
Locally generated installers are unsigned; release signing remains a separate step.

## Current limits

The native paths for navigation, rename/move, batch paste, Recycle Bin/undo,
archive operations, rich previews, custom actions, external providers and
verified updates are implemented in this branch. They still require the
isolated Windows GUI smoke, fault-injection matrix and clean-machine evidence
before being called release-verified. See [feature compatibility](windows-feature-compatibility.md)
and [port milestones](windows-port.md).
