# 04 — Dual pane / split view thực sự

**Trạng thái Strata (source review): THIẾU** · **Ưu tiên: P0** · **Design-only — chưa có implementation/Windows GUI acceptance mới**.

## Flea thực sự triển khai thế nào?
Flea GPUI dành module dual riêng cho divider, focus, selection, activation và session. Hai pane hiển thị thư mục độc lập, không phải browser + preview.

### Bằng chứng Flea ghim commit 73f3c239
- [mod.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/crates/flea-gui/src/dual/mod.rs)
- [session.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/crates/flea-gui/src/dual/session.rs)
- [focus.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/crates/flea-gui/src/dual/focus.rs)
- [layout.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/crates/flea-gui/src/dual/layout.rs)

## Hiện trạng và khoảng trống của Strata Windows
Strata hiện có split sidebar/browser/preview, KHÔNG phải 2 browser độc lập. Need avoid reusing preview pane slot as second browser because semantics/focus differ.

### Bằng chứng Strata ghim commit 35b24d9a
- [layout.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/ui/window/composition/layout.rs)
- [browser_modes.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/ui/browser_modes.rs)
- [tabs_session.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/ui/tabs_session.rs)
- [transfer.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/ui/browser/transfer.rs)
- [architecture.md](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/docs/architecture.md)

## Thiết kế Windows-native / tích hợp vào code hiện tại
Model mỗi TabSession có left BrowserSession và optional right BrowserSession { location, viewMode, navigationPath, focusedItem, selectedItems, scroll, search, preview }. GTK parent split ở window composition, trong mỗi browser vẫn giữ preview drawer riêng nếu enable. One ActivePaneId drives keyboard/commands; operation dispatcher takes immutable source/destination identities; target chooser ưu tiên focused pane, explicit 'copy/move to other pane' command; preserve Windows clipboard semantics.

## Chia lát triển khai
MVP: Ctrl+backslash toggle split, chọn directory bên phải, click đổi active focus, separate navigation/refresh/history, copy path, explicit Copy/Move to other pane with conflict dialog; persist split ratio and last active pane per-tab only after safe restore. Later tab drag/drop and two-pane compare/preview integration.

## Security, correctness, performance, cancel
Hủy browse khi pane đóng; khởi tạo pane ẩn không quét nền; branch-specific move across volumes verify before source deletion; do not let pointer hover override keyboard active pane; no accidental duplicate undo or cross-pane paste target.

## Tiêu chí nghiệm thu trước merge implementation
Hai folder khác nhau độc lập, operations work both directions, Ctrl+Tab only changes tab not pane, active pane visual/accessible, split ratio survives restart, source unchanged on failure/cancel, single-pane regression remains green.

## Quy ước chung
Áp dụng [cross-cutting architecture](00-cross-cutting-architecture.md): GTK không làm blocking I/O; mọi request có generation và hủy; log không chứa path/secret; dùng code Windows thay cho Linux-only APIs; kiểm thử native Windows và negative cases trước khi tuyên bố shipped. Đọc [roadmap](00-roadmap.md) để tránh triển khai trùng các tính năng đã có.
