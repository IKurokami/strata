# 02 — Preview pane gắn cạnh browser

**Trạng thái Strata (source review): ĐÃ CÓ / DUY TRÌ** · **Ưu tiên: P2** · **Design-only — chưa có implementation/Windows GUI acceptance mới**.

## Flea thực sự triển khai thế nào?
Flea bố trí preview alongside column và chỉ tạo thumbnail/renderer cho viewport/request hiện tại; preserve old frame khi trạng thái mới chưa sẵn sàng; loại preview được điều phối theo MIME.

### Bằng chứng Flea ghim commit 73f3c239
- [the-preview-swap.md](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/docs/architecture/the-preview-swap.md)
- [preview_host.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/crates/flea-gui/src/columns/preview_host.rs)
- [thumbnail-requests.md](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/docs/architecture/thumbnail-requests.md)

## Hiện trạng và khoảng trống của Strata Windows
Strata đã có gtk::Paned preview drawer và rule Miller Columns 300 px; rủi ro ở lifecycle, focus, thay đổi selection, async job và phân bổ width chứ không thiếu tính năng.

### Bằng chứng Strata ghim commit 35b24d9a
- [preview-panel-layout.md](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/docs/preview-panel-layout.md)
- [layout.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/ui/window/composition/layout.rs)
- [preview.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/ui/preview.rs)
- [preview.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/services/preview.rs)

## Thiết kế Windows-native / tích hợp vào code hiện tại
Tiếp tục dùng PreviewRequestId/LoadHandle và slot trong gtk::Paned; định nghĩa state Hidden → Reserved → Loading → Ready/Failed, phân biệt disabled/dismissed; preserve resize width và per-tab preview preference; no model reflow per animation frame.

## Chia lát triển khai
Chỉ chỉnh code khi test tái hiện lỗi; thêm targeted regression cho stale preview/menu secondary click/dismissal và parser cancellation; không rewrite preview widget.

## Security, correctness, performance, cancel
Network latency, locked file, GIF/video decoder hủy muộn, monitor callback đi vào tab đã đóng. Giới hạn thumbnail, phân biệt MIME untrusted, giữ sandbox helper fail closed.

## Tiêu chí nghiệm thu trước merge implementation
Preview tự cập nhật theo bàn phím, right-click không thay target, tab đổi không share sai selection, đóng pane không chờ decoder, preview không gây I/O đồng bộ GTK; geometry xác nhận bằng manual capture không layout assertions.

## Quy ước chung
Áp dụng [cross-cutting architecture](00-cross-cutting-architecture.md): GTK không làm blocking I/O; mọi request có generation và hủy; log không chứa path/secret; dùng code Windows thay cho Linux-only APIs; kiểm thử native Windows và negative cases trước khi tuyên bố shipped. Đọc [roadmap](00-roadmap.md) để tránh triển khai trùng các tính năng đã có.
