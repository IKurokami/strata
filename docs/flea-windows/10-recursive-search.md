# 10 — Recursive search as you type

**Trạng thái Strata (source review): ĐÃ CÓ / DUY TRÌ** · **Ưu tiên: P2** · **Design-only — chưa có implementation/Windows GUI acceptance mới**.

## Flea thực sự triển khai thế nào?
Flea scan tree theo lát nhỏ, session compact names/parents, fuzzy scorer, watcher delta, generation/cancel; không stat mọi file ngay từ đầu.

### Bằng chứng Flea ghim commit 73f3c239
- [search.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/src/backend/search.rs)
- [searchindex.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/src/backend/searchindex.rs)
- [searchwatch.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/src/backend/searchwatch.rs)
- [searchscore.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/src/backend/searchscore.rs)

## Hiện trạng và khoảng trống của Strata Windows
Strata có recursive filename search worker + in-pane filtering và global Ctrl+K; không thiếu core behavior. Có thể tối ưu search-as-type trên share/100k files mà không thêm index nền bắt buộc.

### Bằng chứng Strata ghim commit 35b24d9a
- [search.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/services/search.rs)
- [search.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/ui/browser/columns/search.rs)
- [collection.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/ui/browser/collection.rs)
- [smart-search.md](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/docs/smart-search.md)

## Thiết kế Windows-native / tích hợp vào code hiện tại
Preserve worker and PathMatcher; latest-query mailbox cancels prior generation; direct results publish partial, rank deterministic, cap traversal/time/memory and excludes; use Windows watcher only if it proves value, overflow full bounded rescan. File/dir identity mapping must survive rename without wrong selection.

## Chia lát triển khai
MVP chỉ perf/correctness audits; add target input debouncing only if measurement, not arbitrary delay; optional watch incremental index after test on local NTFS and UNC.

## Security, correctness, performance, cancel
Windows reparse/junction loops, network request storms, hidden/system folder disclosure, filename Unicode normalization and 8.3 aliases, rapid user input; no admin-only filesystem index.

## Tiêu chí nghiệm thu trước merge implementation
Search filename as typed, cancel query mid-scan and show latest query only, no freeze for 100k-file fixture or disconnected UNC, correct Unicode Vietnamese queries and result reveal.

## Quy ước chung
Áp dụng [cross-cutting architecture](00-cross-cutting-architecture.md): GTK không làm blocking I/O; mọi request có generation và hủy; log không chứa path/secret; dùng code Windows thay cho Linux-only APIs; kiểm thử native Windows và negative cases trước khi tuyên bố shipped. Đọc [roadmap](00-roadmap.md) để tránh triển khai trùng các tính năng đã có.
