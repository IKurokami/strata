# 07 — Office document preview

**Trạng thái Strata (source review): MỘT PHẦN** · **Ưu tiên: P1** · **Design-only — chưa có implementation/Windows GUI acceptance mới**.

## Flea thực sự triển khai thế nào?
Flea đưa tài liệu Office vào headless LibreOffice 'soffice' trong jail, render PDF kết quả bằng pipeline PDF hiện hữu; bảng có sheet CSV cache riêng. Một job conversion tại một thời điểm và timeout (tham khảo Flea: 60s).

### Bằng chứng Flea ghim commit 73f3c239
- [officepdf.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/src/backend/officepdf.rs)
- [officesheets.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/src/backend/officesheets.rs)
- [mod.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/crates/flea-gui/src/g2/pv2/office/mod.rs)

## Hiện trạng và khoảng trống của Strata Windows
Strata DOCX render subset trong native GTK, XLS/XLSX/ODS Calamine first worksheet; formatting/charts/slide PPTX/full workbook chưa parity high fidelity. Đây là 'một phần' chứ không 'thiếu Office'.

### Bằng chứng Strata ghim commit 35b24d9a
- [document-previews.md](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/docs/document-previews.md)
- [docx.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/services/docx.rs)
- [table.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/services/table.rs)
- [document_media.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/sandbox_helper/document_media.rs)

## Thiết kế Windows-native / tích hợp vào code hiện tại
Thiết kế OfficePreviewProvider 2 tier: lightweight native default cho text/table; optional high-fidelity conversion to PDF using explicit discovered LibreOffice executable in contained helper, private user profile, non-network temp, sandbox review. Cache keyed stable identity/revision + converter version + target/scale, không ghi cache gần file nguồn. XLSX sheet navigation có thể mở bằng Calamine trước khi thêm LibreOffice.

## Chia lát triển khai
MVP: UI sheet switch cho XLSX và PowerPoint PPTX fallback 'open externally' kèm thông báo. Opt-in fidelity backend sau khi phân tích giấy phép, package size và isolation; không auto install LibreOffice.

## Security, correctness, performance, cancel
OOXML ZIP bombs/macros/external links, on-load scripts, volatile formulas, network retrieval, resource exhaustion, personal document cache. Job Objects only resource caps; untrusted office conversion must have real isolation or default off.

## Tiêu chí nghiệm thu trước merge implementation
DOCX/xlsx hiện hữu không regress, clear subset limitations, UI sheet navigation, slide PDF pages if backend enabled, no macro/network, no stale preview, timeout and cleanup, fonts missing -> readable notice.

## Quy ước chung
Áp dụng [cross-cutting architecture](00-cross-cutting-architecture.md): GTK không làm blocking I/O; mọi request có generation và hủy; log không chứa path/secret; dùng code Windows thay cho Linux-only APIs; kiểm thử native Windows và negative cases trước khi tuyên bố shipped. Đọc [roadmap](00-roadmap.md) để tránh triển khai trùng các tính năng đã có.
