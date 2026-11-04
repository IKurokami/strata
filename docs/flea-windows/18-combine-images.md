# 18 — Combine images / contact sheet

**Trạng thái Strata (source review): THIẾU** · **Ưu tiên: P1** · **Design-only — chưa có implementation/Windows GUI acceptance mới**.

## Flea thực sự triển khai thế nào?
Flea dùng composition math chung cho GUI/TUI, ImageMagick nhận images/staged inputs rồi xuất một file layout {vertical,horizontal,grid}, khoảng cách/background, columns; output stage và publish an toàn.

### Bằng chứng Flea ghim commit 73f3c239
- [combine.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/src/backend/combine.rs)
- [printconvert.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/src/backend/printconvert.rs)
- [combine.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/crates/flea-gui/src/g2/op4/combine.rs)
- [combine.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/src/tui/combine.rs)

## Hiện trạng và khoảng trống của Strata Windows
Strata render ảnh/convert nhưng chưa có lệnh Combine built-in. Đây là feature mới có thể dùng TransformJob của mục 17, không phải chỉnh preview.

### Bằng chứng Strata ghim commit 35b24d9a
- [image_conversion.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/services/image_conversion.rs)
- [operations.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/services/operations.rs)
- [preview.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/ui/preview.rs)
- [image_conversion.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/ui/chooser/image_conversion.rs)

## Thiết kế Windows-native / tích hợp vào code hiện tại
ImageCompositionPlan: selected immutable ordered list, layout grid/horizontal/vertical, nCols, spacing, background/alpha, fit/scale, output dimensions compute before decode, target format. User drags ordering, gets lightweight draft preview from existing thumbnail cache. Worker decode bounded images, compose output per tile avoiding giant resident bitmap if possible, encode in isolated helper, no-clobber publish into current folder.

## Chia lát triển khai
MVP: 2–20 JPEG/PNG/WebP into PNG grid with padding/background and explicit filename. Later WebP/JPEG, rows/columns/manual reorder and page contact sheet; no arbitrary URL inputs.

## Security, correctness, performance, cancel
Multiplicative pixel bomb even if individual images legal; bound sum of pixels and output width×height and memory, handle transparency/EXIF orientation, selection deleted, cancellation/temp cleanup, no overwrite.

## Tiêu chí nghiệm thu trước merge implementation
Two-image horizontal/vertical and 9-image grid compare expected dimensions/order/transparency, invalid layout prevents run, input files unchanged, cancel at tile N no output published, UI preview matches generated result's order.

## Quy ước chung
Áp dụng [cross-cutting architecture](00-cross-cutting-architecture.md): GTK không làm blocking I/O; mọi request có generation và hủy; log không chứa path/secret; dùng code Windows thay cho Linux-only APIs; kiểm thử native Windows và negative cases trước khi tuyên bố shipped. Đọc [roadmap](00-roadmap.md) để tránh triển khai trùng các tính năng đã có.
