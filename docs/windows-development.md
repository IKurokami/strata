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

## Current limits

The native paths for navigation, rename/move, batch paste, Recycle Bin/undo,
archive operations, rich previews, custom actions, external providers and
verified updates are implemented in this branch. They still require the
isolated Windows GUI smoke, fault-injection matrix and clean-machine evidence
before being called release-verified. See [feature compatibility](windows-feature-compatibility.md)
and [port milestones](windows-port.md).
