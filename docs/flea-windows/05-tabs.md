# 05 — Tabs

**Trạng thái Strata (source review): ĐÃ CÓ / DUY TRÌ** · **Ưu tiên: P2** · **Design-only — chưa có implementation/Windows GUI acceptance mới**.

## Flea thực sự triển khai thế nào?
Flea tab/shelf session giữ trạng thái cục bộ từng tab và khôi phục state. UI nhanh nhờ loading lazy, không render full preview khi tab không visible.

### Bằng chứng Flea ghim commit 73f3c239
- [rail_rest_tab.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/crates/flea-gui/src/g2/dg2/rail_rest_tab.rs)
- [tab_drop.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/crates/flea-gui/src/g2/dg2/tab_drop.rs)
- [the-state-file.md](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/docs/architecture/the-state-file.md)

## Hiện trạng và khoảng trống của Strata Windows
Strata đã có nhiều tabs, reorder, restore saved locations và drag/drop tab. Không có lý do thay implementation chỉ để giống Flea.

### Bằng chứng Strata ghim commit 35b24d9a
- [tabs_session.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/ui/tabs_session.rs)
- [keyboard-navigation.md](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/docs/keyboard-navigation.md)
- [window.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/ui/window.rs)

## Thiết kế Windows-native / tích hợp vào code hiện tại
Xác định TabSession là sở hữu browser session(s), selected path, history, preview/search, cancellation tokens; focus active tab nhất quán với dual-pane. Persist only safe local directory locations; secret-containing URI, credential, temporary trash items không viết session.

## Chia lát triển khai
MVP cho feature này chỉ cải tiến nếu regression chứng minh lỗi; trước khi 04 dual-pane triển khai, thêm tests tab isolation và job lifecycle.

## Security, correctness, performance, cancel
Đóng tab có operation đang chạy phải follow close guard; không leak credentials/remote connection; stale tab worker không update window sau close.

## Tiêu chí nghiệm thu trước merge implementation
Ctrl+T/Ctrl+W, tab ordering, restore và clipboard target ổn định; tab B không nhận reply tab A; Windows Shell reveal reuse đúng tab; no hidden tab thumbnail sweep.

## Quy ước chung
Áp dụng [cross-cutting architecture](00-cross-cutting-architecture.md): GTK không làm blocking I/O; mọi request có generation và hủy; log không chứa path/secret; dùng code Windows thay cho Linux-only APIs; kiểm thử native Windows và negative cases trước khi tuyên bố shipped. Đọc [roadmap](00-roadmap.md) để tránh triển khai trùng các tính năng đã có.
