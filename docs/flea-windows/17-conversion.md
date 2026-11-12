# 17 — Convert formats (single and batch)

**Trạng thái Strata: MỘT PHẦN** · **Ưu tiên: P1**. Trình duyệt đã có lệnh
[chuyển một hoặc nhiều ảnh sang PNG, WebP lossless hoặc JPEG](../image-exports.md).
Các định dạng đích khác vẫn thuộc thiết kế bên dưới.

## Flea thực sự triển khai thế nào?
Flea có Convert UI, classifier theo MIME, ảnh convert qua ImageMagick 'magick' (đuôi target chọn encoder) và optional strip metadata; staged private work+batch progress, avoid overwrite. Đây là external-tool orchestration.

### Bằng chứng Flea ghim commit 73f3c239
- [convert.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/src/backend/convert.rs)
- [convertbatch.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/src/backend/convertbatch.rs)
- [printconvert.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/src/backend/printconvert.rs)
- [convert.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/crates/flea-gui/src/g2/op4/convert.rs)

## Hiện trạng và khoảng trống của Strata Windows
Strata hiện có conversion các format ảnh sang PNG cho file chooser và custom action recipes; chưa có general context menu batch output-format conversion như Flea.

### Bằng chứng Strata ghim commit 35b24d9a
- [image_conversion.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/services/image_conversion.rs)
- [image_conversion.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/ui/chooser/image_conversion.rs)
- [custom-actions.md](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/docs/custom-actions.md)
- [image_conversion.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/sandbox_helper/image_conversion.rs)

## Thiết kế Windows-native / tích hợp vào code hiện tại
TransformRegistry (source MIME signature → permitted outputs), ConvertJob input identity + output target + settings + cancellation; plug built-in image crate/helper for JPEG/PNG/WebP first; optional FFmpeg/video & LibreOffice document converter only if licensed and isolated. Preview estimated outputs and naming collisions; stage in destination volume, no-clobber publish, preserve originals, explicit metadata preservation/strip policy; show job progress in existing dock.

## Chia lát triển khai
MVP: JPEG↔PNG↔WebP (respect animated inputs: refuse or warn first-frame) in one/batch, quality target and metadata policy. Add PDF page export, media formats and Office only with tested helpers; don't silently use arbitrary magick from PATH.

## Security, correctness, performance, cancel
Malicious codecs, EXIF/private metadata, decompression bombs, huge dimensions, filenames clashes, palette/alpha loss, HDR/color profile, source extension spoofing; decoding must use contained helper and resource limits, cancellation safe.

## Tiêu chí nghiệm thu trước merge implementation
Fixture JPEG/PNG/WebP round-trip, alpha/ICC/EXIF choices, output name conflict + overwrite refusal, 50-file batch with failed item/undo policy, canceled batch keeps every source, no unexpectedly executed external process.

## Quy ước chung
Áp dụng [cross-cutting architecture](00-cross-cutting-architecture.md): GTK không làm blocking I/O; mọi request có generation và hủy; log không chứa path/secret; dùng code Windows thay cho Linux-only APIs; kiểm thử native Windows và negative cases trước khi tuyên bố shipped. Đọc [roadmap](00-roadmap.md) để tránh triển khai trùng các tính năng đã có.
