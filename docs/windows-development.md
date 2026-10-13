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
pacman -S --needed mingw-w64-ucrt-x86_64-{rust,gcc,clang,pkgconf,gtk4,gtksourceview5,gstreamer,gst-plugins-base,poppler,glib2,fontconfig,cmake}
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

The test action runs native I/O tests and the eleven non-GUI Windows safety
regressions, requiring each regression to exist. GTK interaction tests still
need a separately isolated display/session; this command does not run them on
the user's desktop. A successful `--version` alone does not prove first paint.

The package action creates a fresh timestamped directory under `target/`.
Copy or extract the complete directory, then launch `Start-Strata.cmd`.
The executable depends on the adjacent `bin`, `lib`, `libexec`, `share`, and
`etc` directories. The package smoke checks exclude MSYS2 from PATH.

## Current limits

This is a developer build. Local navigation, filename search, new files/folders,
and no-overwrite single-file copy are available. Rename/move, batch paste,
Recycle Bin/undo, archive operations, rich previews, custom action execution,
external providers, and automatic update installation remain incomplete or
explicitly unavailable on Windows. See [feature compatibility](windows-feature-compatibility.md)
and [port milestones](windows-port.md). Do not report these features as verified
because the executable compiles or the native I/O tests pass.
