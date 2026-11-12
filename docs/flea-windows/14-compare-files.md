# 14 — Compare files / text diff

**Trạng thái Strata (source review): THIẾU NATIVE** · **Ưu tiên: P0** · **Design-only — chưa có implementation/Windows GUI acceptance mới**.

## Flea thực sự triển khai thế nào?
Flea chỉ so sánh **hai regular text files** ra unified diff (sử dụng /usr/bin/diff trong jail), giới hạn <=1MiB mỗi input, NUL 8KiB đầu coi là binary, <=8MiB output. GUI có Quick Look compare pane, không phải full binary diff editor.

### Bằng chứng Flea ghim commit 73f3c239
- [compare.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/src/backend/compare.rs)
- [quick_look_compare.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/crates/flea-gui/src/window/quick_look_compare.rs)
- [card_diff.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/crates/flea-gui/src/g2/pv1/card_diff.rs)

## Hiện trạng và khoảng trống của Strata Windows
Strata có preview text/SourceView/selection, chưa thấy native comparison command với exactly two selected files. Không thêm external diff tool không rõ nguồn vào Windows distribution.

### Bằng chứng Strata ghim commit 35b24d9a
- [preview.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/ui/preview.rs)
- [virtual_preview.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/ui/virtual_preview.rs)
- [preview.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/ui/keys/bindings/preview.rs)
- [context_menu.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/ui/browser/context_menu.rs)

## Thiết kế Windows-native / tích hợp vào code hiện tại
CompareProvider nhận 2 FileIdentity snapshot; detect text/binary/encoding/line-endings trong read-only worker. Dùng Rust diff algorithm có cap line count, byte limit, time; return immutable DiffHunk {oldRange,newRange,lines,kind}; GTK compare viewer reuse virtual preview/source highlighting, unified as MVP, side-by-side later. Preserve two source previews and navigational context; menu action only enabled for exactly two regular local files.

## Chia lát triển khai
MVP: compare two UTF-8 text files <= configured bound, lines changed/added/removed and copy hunk; hex digest equality for binary can be separate clearly labeled result, never claim full binary diff.

## Security, correctness, performance, cancel
No run diff.exe from PATH, no saving/overwriting, symlink/reparse refusal or safe identity reopen, Unicode normalization and CRLF deterministic, large diff refusal with 'open externally' fallback, cancellation on close/navigation.

## Tiêu chí nghiệm thu trước merge implementation
Fixtures identical/added/deleted/reordered/CRLF/non-UTF8/binary/oversized; selection invalid disables action; hunk line numbers match files; UI cannot mutate files; cancellation doesn't flash diff of old pair.

## Quy ước chung
Áp dụng [cross-cutting architecture](00-cross-cutting-architecture.md): GTK không làm blocking I/O; mọi request có generation và hủy; log không chứa path/secret; dùng code Windows thay cho Linux-only APIs; kiểm thử native Windows và negative cases trước khi tuyên bố shipped. Đọc [roadmap](00-roadmap.md) để tránh triển khai trùng các tính năng đã có.
