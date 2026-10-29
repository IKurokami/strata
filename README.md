<div align="center">

<img src="docs/assets/logos/strata-tokyo-night.svg" alt="Strata logo" width="120">

# Strata for Windows

**A keyboard-friendly file manager with tabs, rich previews and Windows Shell integration.**

[![Windows](https://img.shields.io/badge/Windows-10%20%2F%2011-0078D4)](#installation)
[![Latest stable](https://img.shields.io/github/v/release/IKurokami/strata?display_name=tag)](https://github.com/IKurokami/strata/releases/latest)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

<img src="docs/assets/windows/columns.png" alt="Strata running on Windows with column navigation and a Markdown preview" width="1280">

</div>

This is a **Windows-focused fork** of [LGSE/Strata](https://github.com/lgse/strata).
It keeps Strata’s GTK4 interface and keyboard navigation, adds Windows integration,
and ships a bundled native runtime. Releases and the Windows updater use
**IKurokami/strata**. The upstream project remains the reference for Linux and
Omarchy development.

## Installation

Download the Windows x64 installer from [the latest stable release](https://github.com/IKurokami/strata/releases/latest).
Run `Strata-1.0.1-windows-x64-setup.exe`. Installation is per-user and does not
require MSYS2, Rust or administrator access.

For portable use, download `strata-1.0.1-x86_64-pc-windows-gnu.zip`, extract the
**entire archive**, and run `Start-Strata.exe`. Keep its `bin`, `lib` and `share`
folders together. Copying only `strata.exe` is not a portable installation.

### Updates

Use **Settings → Updates** to check for new versions and install updates.
Stable releases are the default channel; enable prerelease updates to try RCs.
Windows update archives have an Ed25519-signed manifest and a verified SHA-256
checksum. Each release also includes `SHA256SUMS` and a standalone installer.

## Features

- **One window, many tabs.** Columns, Icons, List and Disk Usage views share navigation, selection and file actions.
- **Windows integration.** Open files with their associated applications, use Open with and native Properties, and exchange local files with Explorer and other apps through clipboard and drag-and-drop.
- **Rich previews.** Images, animated GIF/WebP, PDF, text, source code, Markdown, audio and video. Video includes timeline previews and scrubbing; large text previews are bounded.
- **Discord text editing.** Fenced ANSI text renders in color immediately. Edit ANSI and Markdown inside the preview panel, with selection tools, visual colors, gradients, undo/redo and explicit copy/save. See [supported formats and limits](docs/discord-editor.md).
- **Storage views.** This PC shows drives and capacity. Disk Usage scans in the background and presents a stable map while scanning. See [Disk Usage](docs/disk-usage.md).
- **Local search.** Fast filename search plus local semantic image/text search after the optional model download. Models run locally; see [Smart Search](docs/smart-search.md).
- **Archive extraction.** Grouped extraction actions, extraction into a same-named folder, password prompts and the Windows archive backend. Supported formats include ZIP, 7z, TAR and RAR.
- **Appearance and preferences.** Light/dark themes, macOS Classic themes, colored Material file icons, Windows font selection, and remembered folder sorting/view choices.
- **Background access.** A tray icon, optional background residency, and manual cache clearing from Settings or the tray.

<img src="docs/assets/windows/settings.png" alt="Windows Strata Settings with the shared language selector" width="1000">

<img src="docs/assets/windows/language-menu.png" alt="The compact language dropdown, with the current language selected" width="220">

### Windows Shell options

The **Replace File Explorer** option registers Strata for folder/drive opens and
Win+E. Requests are routed into tabs in the existing Strata window. This is
optional; disabling the option or uninstalling restores the previous registration.

**Delete using Windows** requests the Recycle Bin, like ordinary Explorer Delete.
Permanent deletion is a separate action. Windows Properties can be selected in
Settings and remains available from the Open in menu.

Native integrations cover local Windows paths. Remote locations and individual
Shell extensions can have different capabilities.

## Getting around

| Action | Shortcut |
| --- | --- |
| New tab | Ctrl+T |
| Search | Ctrl+K |
| Preview selected file | Space |
| Shortcut reference | F1 |

Use the in-app shortcut reference for the active keyboard map. See
[keyboard navigation](docs/keyboard-navigation.md) and
[preview panel behavior](docs/preview-panel-layout.md) for more detail.

## Development on Windows

Use the native **MSYS2 UCRT64** toolchain and the project’s private GTK/GStreamer
runtime. Start with [Windows development](docs/windows-development.md) for the
required packages, environment and runtime build instructions.

From PowerShell in the checkout:

```powershell
.\scripts\windows\dev.ps1 -Action build
.\scripts\windows\dev.ps1 -Action run
```

The development helper builds the application and Shell helper and stages the
private runtime. Optimized builds and packaging are documented in
[Windows development](docs/windows-development.md) and
[the Windows runtime notes](packaging/windows/runtime/README.md).
The packaged application includes the required GTK/GStreamer libraries, plugins
and FFmpeg tools.

Additional references:

- [Contributing](CONTRIBUTING.md)
- [Architecture](docs/architecture.md)
- [Saved preferences](docs/preferences.md)
- [Custom actions](docs/custom-actions.md)
- [Themes](docs/themes.md)
- [Release process](docs/releasing.md)

## Upstream and credits

The original Strata project is developed by [LGSE and its contributors](https://github.com/lgse/strata/graphs/contributors).
This fork focuses on Windows compatibility, integration and distribution, and
selectively brings in useful upstream fixes. For Linux installation and desktop
integration, use [the upstream documentation](https://github.com/lgse/strata#installation).

## License

Strata source is distributed under the [MIT License](LICENSE). Bundled runtime
libraries, fonts, icons and optional archive components retain their own licenses
and notices; see [THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md).
