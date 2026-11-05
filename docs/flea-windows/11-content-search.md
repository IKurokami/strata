# 11 — File content search

**Trạng thái Strata (source review): CÓ / GIỚI HẠN THEO INDEX** · **Ưu tiên: P1** · **Design-only — chưa có implementation/Windows GUI acceptance mới**.

## Flea thực sự triển khai thế nào?
Flea dùng external ripgrep 'rg' dưới jailed read-only process cho live content search, one serialized worker, query generation, output cap 64MiB và 30s wall limit; tên files search path riêng.

### Bằng chứng Flea ghim commit 73f3c239
- [contents.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/src/backend/contents.rs)
- [contentsworker.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/src/backend/contentsworker.rs)
- [contentsparse.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/src/backend/contentsparse.rs)
- [contents.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/crates/flea-gui/src/g2/sr1/contents.rs)

## Hiện trạng và khoảng trống của Strata Windows
Strata đã có FTS5 indexed content + semantic local opt-in; nếu thư mục chưa index thì không có parity với Flea kiểu ad-hoc scan toàn subtree. Đừng vô tình khiến index mọi file hoặc download AI models tự động.

### Bằng chứng Strata ghim commit 35b24d9a
- [smart-search.md](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/docs/smart-search.md)
- [smart_search.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/services/smart_search.rs)
- [store.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/services/smart_search/store.rs)
- [indexer.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/services/smart_search/indexer.rs)
- [smart.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/ui/search/smart.rs)

## Thiết kế Windows-native / tích hợp vào code hiện tại
Cung cấp SearchMode { filename, indexed-content, live-content } trên existing UI. Live mode cho root user-selected, có explicit resource budgets, guarded file types / binary skip, hỗ trợ ripgrep if bundled+verified hoặc Rust bounded walker + text extractor; adapter returns SearchResult với content location hợp PreviewRegistry, only most recent query emitted. Index mode vẫn ưu tiên khi roots đã opt-in.

## Chia lát triển khai
MVP: nhãn phân biệt Indexed vs Search this folder now, live search plain-text UTF-8 và result excerpt/cancel. Office/PDF live extraction chỉ sau sandbox isolation; semantic search độc lập.

## Security, correctness, performance, cancel
No automatic scanning home/drive, no content logging, honor symlink/reparse/ignore policies, avoid long/locked/hidden files and UNC storms, no command shell. Tránh search index dữ liệu nhạy cảm không có user opt-in.

## Tiêu chí nghiệm thu trước merge implementation
Tìm từ trong file text ở root chưa index, user cancel + type query khác không leak hits cũ; indexed search không đổi ranking, path secrets không log, result click highlight file/line, binary+oversized skip được giải thích.

## Quy ước chung
Áp dụng [cross-cutting architecture](00-cross-cutting-architecture.md): GTK không làm blocking I/O; mọi request có generation và hủy; log không chứa path/secret; dùng code Windows thay cho Linux-only APIs; kiểm thử native Windows và negative cases trước khi tuyên bố shipped. Đọc [roadmap](00-roadmap.md) để tránh triển khai trùng các tính năng đã có.
