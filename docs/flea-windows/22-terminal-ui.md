# 22 — Terminal UI (TUI) version

**Trạng thái Strata (source review): THIẾU** · **Ưu tiên: P3** · **Design-only — chưa có implementation/Windows GUI acceptance mới**.

## Flea thực sự triển khai thế nào?
Flea CLI có mode --tui với full Rust terminal view riêng, reuse backend protocol for actions/search/listings. Đây không chỉ là 'Open in terminal' button.

### Bằng chứng Flea ghim commit 73f3c239
- [mod.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/src/tui/mod.rs)
- [model.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/src/tui/model.rs)
- [pane.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/src/tui/pane.rs)
- [preview.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/src/tui/preview.rs)
- [contents.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/src/tui/contents.rs)

## Hiện trạng và khoảng trống của Strata Windows
Strata Windows có launcher wt.exe/PowerShell/cmd tại thư mục; không có interactive file manager trong terminal. TUI cần product app riêng và factoring core tránh dependency GTK.

### Bằng chứng Strata ghim commit 35b24d9a
- [terminal.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/ui/terminal.rs)
- [operations.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/services/operations.rs)
- [search.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/services/search.rs)
- [browser.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/app/browser.rs)

## Thiết kế Windows-native / tích hợp vào code hiện tại
Tạo strata-tui.exe dùng ratatui+crossterm hoặc equivalent with Windows Terminal UTF-8 capability, alternate screen, resize and mouse events opt-in, panic-restored terminal state. Extract platform-neutral browser commands/selection/history/search/operation service interface from GTK dependencies, consume same native adapters; avoid second inconsistent file-op implementation. Keyboard profiles can share action IDs, not key event objects. Preview TUI text, ANSI image only optional.

## Chia lát triển khai
MVP after core extraction: two-column/one-pane list, navigation, filter/search, copy/move/trash with explicit confirm/undo, terminal restore on crash; no video/Office/3D promised. Later multi tab and dual pane, remote providers.

## Security, correctness, performance, cancel
Windows conhost vs Windows Terminal, non-UTF8 paths display, signal/CTRL+C cancel, terminal escape injection in filenames, nested terminal, clipboard secure operations, sync IO stalling frame, ANSI OSC injection; make TUI opt-in separately packaged.

## Tiêu chí nghiệm thu trước merge implementation
Start/exit restores terminal, no lingering raw mode after panic, Unicode filename escaped safely, directory 100k listing usable, F2/rename/recycle via shared core, copy failure semantics same as GUI, packaging without GTK-only dependencies for TUI binary where possible.

## Quy ước chung
Áp dụng [cross-cutting architecture](00-cross-cutting-architecture.md): GTK không làm blocking I/O; mọi request có generation và hủy; log không chứa path/secret; dùng code Windows thay cho Linux-only APIs; kiểm thử native Windows và negative cases trước khi tuyên bố shipped. Đọc [roadmap](00-roadmap.md) để tránh triển khai trùng các tính năng đã có.
