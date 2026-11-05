# 01 — Quick Look (Space và full window)

**Trạng thái Strata (source review): MỘT PHẦN** · **Ưu tiên: P2** · **Design-only — chưa có implementation/Windows GUI acceptance mới**.

## Flea thực sự triển khai thế nào?
Flea giữ Quick Look thành surface riêng trên GPUI, đồng bộ kiểu preview với panel, và có cơ chế giữ khung cũ trong lúc renderer mới chuẩn bị; backend không block cursor. Đây là GPUI hiện hành, khác QML PreviewSwap lịch sử.

### Bằng chứng Flea ghim commit 73f3c239
- [look.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/crates/flea-gui/src/g2/pv1/look.rs)
- [quick_look_compare.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/crates/flea-gui/src/window/quick_look_compare.rs)
- [the-preview-swap.md](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/docs/architecture/the-preview-swap.md)

## Hiện trạng và khoảng trống của Strata Windows
Strata đã có Space để xem nhanh cùng preview drawer; tại snapshot chưa đủ bằng chứng có chế độ full-window immersive độc lập. Cần kiểm tra tương tác Ctrl/Space, selection/focus và geometry thực tế, không thay behavior Space tùy tiện.

### Bằng chứng Strata ghim commit 35b24d9a
- [README.md](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/README.md)
- [preview.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/ui/preview.rs)
- [preview.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/ui/keys/bindings/preview.rs)
- [preview-panel-layout.md](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/docs/preview-panel-layout.md)

## Thiết kế Windows-native / tích hợp vào code hiện tại
Tạo PreviewSurfaceMode { Inline, Overlay } trên cùng PreviewSession/PreviewRegistry; overlay chiếm vùng content window (không nhất thiết fullscreen OS); Esc trở về vị trí selection/scroll; Ctrl+L, media keys và input focus có thứ tự ưu tiên rõ. Request generation sống qua chuyển mode nhưng mỗi decoded frame phải gắn đúng file revision.

## Chia lát triển khai
MVP: lệnh Open full Quick Look từ menu/shortcut nhất quán, overlay dùng renderer hiện tại cho ảnh/PDF/text/video. Sau đó có filmstrip nhiều lựa chọn, compare side-by-side và detachable viewer nếu có nhu cầu thật.

## Security, correctness, performance, cancel
Không mở decoder đôi cho cùng file; preview cũ giữ cho đến frame sẵn sàng rồi giải phóng; media playback phải pause/stop khi overlay đóng; nếu file mất giữa thao tác, show stale/not found chứ không vẽ nhầm.

## Tiêu chí nghiệm thu trước merge implementation
Space hoạt động như trước; overlay mở/đóng bằng bàn phím/chuột; Esc trả focus đúng tab và selected row, đổi file nhanh 100 lần không hiện frame sai/không rò helper; first preview không chặn GTK main thread.

## Quy ước chung
Áp dụng [cross-cutting architecture](00-cross-cutting-architecture.md): GTK không làm blocking I/O; mọi request có generation và hủy; log không chứa path/secret; dùng code Windows thay cho Linux-only APIs; kiểm thử native Windows và negative cases trước khi tuyên bố shipped. Đọc [roadmap](00-roadmap.md) để tránh triển khai trùng các tính năng đã có.
