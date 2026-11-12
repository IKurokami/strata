# 03 — Miller columns

**Trạng thái Strata (source review): ĐÃ CÓ / DUY TRÌ** · **Ưu tiên: P2** · **Design-only — chưa có implementation/Windows GUI acceptance mới**.

## Flea thực sự triển khai thế nào?
Flea dựng chuỗi các cột theo navigation, listing hai pha tên/type rồi metadata của viewport, cập nhật hàng theo watch; render giữ selection/cursor khi chuyển cột.

### Bằng chứng Flea ghim commit 73f3c239
- [two-phase-listing.md](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/docs/architecture/two-phase-listing.md)
- [preview_host.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/crates/flea-gui/src/columns/preview_host.rs)
- [the-open-directory-is-watched.md](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/docs/architecture/the-open-directory-is-watched.md)

## Hiện trạng và khoảng trống của Strata Windows
Strata Columns đã có, width mặc định 300 px và per-column resize; không nên dựng thêm UI thứ hai. Điểm cần kiểm lại: UNC, directory lớn, focus, preview replacement và copy/drop target.

### Bằng chứng Strata ghim commit 35b24d9a
- [columns.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/ui/browser/columns.rs)
- [preview-panel-layout.md](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/docs/preview-panel-layout.md)
- [keyboard-navigation.md](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/docs/keyboard-navigation.md)
- [rows.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/ui/browser/columns/rows.rs)

## Thiết kế Windows-native / tích hợp vào code hiện tại
Tái dùng NavigationPath và BrowserEvent: active column, open path, selected rows phải khác nhau. Preserve trailing columns và per-column scroll/width; load subfolder asynchronously; một cửa sổ có 2 BrowserSession nếu dual pane sau này.

## Chia lát triển khai
Chỉ harden: refresh overflow, rapid navigation, sort/scroll state, accessibility, Windows file names. Feature 04 dual-pane tạo root container mới, không sửa semantics của Miller Columns.

## Security, correctness, performance, cancel
Không chạy symlink/junction recursive không có guard; không lấy full metadata mọi row; đường UNC offline phải hiển thị partial/unavailable state.

## Tiêu chí nghiệm thu trước merge implementation
Mở cây nhiều cấp, duyệt hàng chục nghìn file, kéo divider, đổi view, copy/paste/rename từ cột focus và quay lại bằng Left không mất scroll hay chọn sai nguồn.

## Quy ước chung
Áp dụng [cross-cutting architecture](00-cross-cutting-architecture.md): GTK không làm blocking I/O; mọi request có generation và hủy; log không chứa path/secret; dùng code Windows thay cho Linux-only APIs; kiểm thử native Windows và negative cases trước khi tuyên bố shipped. Đọc [roadmap](00-roadmap.md) để tránh triển khai trùng các tính năng đã có.
