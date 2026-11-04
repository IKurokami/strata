# 06 — 3D model preview

**Trạng thái Strata (source review): MỘT PHẦN** · **Ưu tiên: P1** · **Design-only — chưa có implementation/Windows GUI acceptance mới**.

## Flea thực sự triển khai thế nào?
Flea có model parser/render-independent dữ liệu (STL, 3MF, G-code/BG-code và các định dạng liên quan PLY/AMF) được load trong thumbnail jail, cap 128 MiB input, 256 MiB expanded, giới hạn components/vertices/plates; GPUI render separate surface.

### Bằng chứng Flea ghim commit 73f3c239
- [mod.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/src/backend/model3d/mod.rs)
- [limits.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/src/backend/model3d/limits.rs)
- [model3dreq.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/src/backend/model3dreq.rs)
- [mod.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/crates/flea-gui/src/g2/pv4/mod.rs)

## Hiện trạng và khoảng trống của Strata Windows
Strata có hỗ trợ STL/3MF/FreeCAD với model preview helper và bounds; không nên viết parser mới cho format đã có. Khoảng trống: các format Flea hỗ trợ thêm, slicer facts/layers, UX controls.

### Bằng chứng Strata ghim commit 35b24d9a
- [model_preview.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/services/model_preview.rs)
- [model.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/sandbox_helper/model.rs)
- [model_preview.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/adapters/local_preview/tests/model_preview.rs)
- [preview-sandbox.md](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/docs/preview-sandbox.md)

## Thiết kế Windows-native / tích hợp vào code hiện tại
Giữ ModelFormat + helper current, thêm registry decoder theo format và metadata normalized mesh với [vertices, triangles, bounds, transform, plate] có version; phân biệt G-code toolpath với solid mesh. No network, no executable content, sniff signature và bounded parse. GPU rendering via existing pipeline or CPU fallback (không ép GPUI).

## Chia lát triển khai
MVP 1: bổ sung ASCII/binary PLY hoặc AMF có fixture malformed và color/vertex caps. MVP 2: G-code metadata/layer preview (nếu người dùng cần 3D printing), sau đó xem xét multi-part/3MF plates; không parse full FreeCAD application macros.

## Security, correctness, performance, cancel
ZIP bombs, XML entities, invalid mesh numbers, NaN/Inf geometry, pathological triangle count, 3MF external resources, stale helper frames; review security isolation stronger than Job Object.

## Tiêu chí nghiệm thu trước merge implementation
STL/3MF/FreeCAD cũ vẫn mở; new supported fixture thực sự render bounds đúng, rotate/zoom/reset, malformed/bomb input fail closed, cancellation releases worker, no file read privilege outside target.

## Quy ước chung
Áp dụng [cross-cutting architecture](00-cross-cutting-architecture.md): GTK không làm blocking I/O; mọi request có generation và hủy; log không chứa path/secret; dùng code Windows thay cho Linux-only APIs; kiểm thử native Windows và negative cases trước khi tuyên bố shipped. Đọc [roadmap](00-roadmap.md) để tránh triển khai trùng các tính năng đã có.
