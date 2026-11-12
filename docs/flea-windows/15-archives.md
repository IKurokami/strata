# 15 — Create and extract archives

**Trạng thái Strata (source review): ĐÃ CÓ / GIA CỐ** · **Ưu tiên: P2** · **Design-only — chưa có implementation/Windows GUI acceptance mới**.

## Flea thực sự triển khai thế nào?
Flea có archive listing, compress/extract/format conversion, cancelled isolated subprocess, private staging + publish no-clobber, control archive types/password, undo/pending job bookkeeping.

### Bằng chứng Flea ghim commit 73f3c239
- [archiveops.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/src/backend/archiveops.rs)
- [archivework.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/src/backend/archivework.rs)
- [archivelist.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/src/backend/archivelist.rs)
- [write-operations-and-the-undo-journal.md](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/docs/architecture/write-operations-and-the-undo-journal.md)

## Hiện trạng và khoảng trống của Strata Windows
Strata đã hỗ trợ tạo/extract ZIP/7z/TAR/RAR theo các khả năng backend và archive preview. Không đổi cả implementation; ưu tiên parity/edge cases và Windows packaging codec/license.

### Bằng chứng Strata ghim commit 35b24d9a
- [archives.md](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/docs/archives.md)
- [archive-extraction.md](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/docs/archive-extraction.md)
- [archive.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/adapters/local_operations_windows/archive.rs)
- [compression.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/adapters/local_operations/archive/compression.rs)
- [archive.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/ui/browser/archive.rs)

## Thiết kế Windows-native / tích hợp vào code hiện tại
Giữ Operations/WindowsArchiveAdapter hiện tại; unify ArchiveJob {List, Compress, Extract} progress/cancel. Preflight cumulative expanded bytes, count/depth, path traversal, link escape, reserved Windows names, ADS targets, duplicate case-insensitive entries. Materialize in ACL-restricted staging and no-clobber publish, conflict UI, preserved source. Browse virtual archive entries read-only.

## Chia lát triển khai
MVP chỉ mở rộng khi bug reproduction: RAR password, multipart, encrypted 7z, symbolic links, Unicode/UNC, archive path root collisions, integrity check. Không hứa codec license không bundled.

## Security, correctness, performance, cancel
ZIP slip, decompression bomb, symlink/junction, archive script hooks (never execute), disk exhausted, cross-volume output and partial publish. Use existing sandbox architecture and sanitize archive path.

## Tiêu chí nghiệm thu trước merge implementation
Create+extract round-trip for supported formats, password refusal/retry, malware-style '../' and absolute paths blocked, NTFS case collision flagged, cancellation cleans staging or reports recoverable partial, no source deletion.

## Quy ước chung
Áp dụng [cross-cutting architecture](00-cross-cutting-architecture.md): GTK không làm blocking I/O; mọi request có generation và hủy; log không chứa path/secret; dùng code Windows thay cho Linux-only APIs; kiểm thử native Windows và negative cases trước khi tuyên bố shipped. Đọc [roadmap](00-roadmap.md) để tránh triển khai trùng các tính năng đã có.
