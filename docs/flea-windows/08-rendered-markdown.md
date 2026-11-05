# 08 — Rendered Markdown

**Trạng thái Strata (source review): ĐÃ CÓ / DUY TRÌ** · **Ưu tiên: P2** · **Design-only — chưa có implementation/Windows GUI acceptance mới**.

## Flea thực sự triển khai thế nào?
Flea v0.4.0 dùng GPUI document/paint, worker và figures renderer (không dùng QML renderer cũ); selection/scroll và media asset lifecycle tách khỏi parsing.

### Bằng chứng Flea ghim commit 73f3c239
- [doc.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/crates/flea-gui/src/g2/pv2/markdown/doc.rs)
- [paint.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/crates/flea-gui/src/g2/pv2/markdown/paint.rs)
- [markdown-figures.md](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/docs/architecture/markdown-figures.md)

## Hiện trạng và khoảng trống của Strata Windows
Strata đã dùng pulldown-cmark, native GTK view, mermaid-svg, TeX notation, images relative path constrained, Rendered/Source mode và có editor. Phải giữ native approach; không thêm webview làm tăng attack surface.

### Bằng chứng Strata ghim commit 35b24d9a
- [document-previews.md](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/docs/document-previews.md)
- [document.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/services/document.rs)
- [virtual_preview.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/ui/virtual_preview.rs)
- [markdown.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/ui/discord_editor/markdown.rs)

## Thiết kế Windows-native / tích hợp vào code hiện tại
Tái dùng existing DocumentLayout/virtual preview; ưu tiên correctness: immutable parsed blocks, bounded parser, relative image canonicalization and symlink checks, source-offset mapping để Ctrl+F/selection. Render math/diagram offline, fallback source khi unsupported.

## Chia lát triển khai
Không viết lại; mỗi enhancement bắt đầu bằng fixture thể hiện thiếu support (GFM code/tables/links/assets), rồi mở renderer có kiểm soát.

## Security, correctness, performance, cancel
Markdown raw HTML/script, remote image beacon, data URL memory payload, relative traversal escaping root, huge document layout or Unicode text selection. Keep all external content inert.

## Tiêu chí nghiệm thu trước merge implementation
Toggle Rendered/Source giữ scroll/selection, long docs virtualize, Mermaid/TeX fallback không crash, image outside root không load, remote content không tải, text copy không sai Unicode.

## Quy ước chung
Áp dụng [cross-cutting architecture](00-cross-cutting-architecture.md): GTK không làm blocking I/O; mọi request có generation và hủy; log không chứa path/secret; dùng code Windows thay cho Linux-only APIs; kiểm thử native Windows và negative cases trước khi tuyên bố shipped. Đọc [roadmap](00-roadmap.md) để tránh triển khai trùng các tính năng đã có.
