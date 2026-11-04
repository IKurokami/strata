# 16 — Print selected files

**Trạng thái Strata (source review): THIẾU NATIVE** · **Ưu tiên: P1** · **Design-only — chưa có implementation/Windows GUI acceptance mới**.

## Flea thực sự triển khai thế nào?
Flea trên Linux query CUPS bằng lpstat/lpoptions và gửi job qua lp, hỗ trợ PDF/image/text/Office; Office được chuyển PDF trước khi in. Một worker làm job và cancel, không block UI.

### Bằng chứng Flea ghim commit 73f3c239
- [printreq.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/src/backend/printreq.rs)
- [printtool.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/src/backend/printtool.rs)
- [printing.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/src/backend/printing.rs)
- [officepdf.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/src/backend/officepdf.rs)

## Hiện trạng và khoảng trống của Strata Windows
Strata có preview nội dung và shell Open With nhưng chưa có Print workflow integrated Windows. CUPS argv Flea không áp dụng trên Windows.

### Bằng chứng Strata ghim commit 35b24d9a
- [preview.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/ui/preview.rs)
- [document.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/services/document.rs)
- [preview.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/services/preview.rs)
- [windows_open.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/ui/windows_open.rs)

## Thiết kế Windows-native / tích hợp vào code hiện tại
PrintController với PrintSource {Pdf, Image, Text, OfficeConverted} và PrinterCapability. Dùng Win32 PrintDlgEx/print spooler APIs ở STA-aware worker hoặc Windows common print experience; render PDF pages qua existing pipeline với DPI bounded. Nếu type không có renderer an toàn, tùy chọn 'Open in associated app to print' nhưng không giả lập job success. Explicit print options: copies/page range/duplex/color when driver reports supported; report job accepted vs printed as two separate states.

## Chia lát triển khai
MVP: PDF and image to default selected printer, print dialog + preview, page range, cancellation before spool submission. Sau đó document text/Office export PDF optional; print multiple files queue bounded.

## Security, correctness, performance, cancel
Printer driver may block/crash, installation/permission/remote printer quirks, format-specific page geometry, PII sent to device; no silent auto-print or printing from untrusted plugin without consent. Force spool on worker, specify timeout where possible.

## Tiêu chí nghiệm thu trước merge implementation
Mock printer capability and local virtual PDF printer fixture, page-count/range validation, selecting canceled leaves no job, duplex shown only when supported, errors surfaced, no GUI freeze on offline network printer.

## Quy ước chung
Áp dụng [cross-cutting architecture](00-cross-cutting-architecture.md): GTK không làm blocking I/O; mọi request có generation và hủy; log không chứa path/secret; dùng code Windows thay cho Linux-only APIs; kiểm thử native Windows và negative cases trước khi tuyên bố shipped. Đọc [roadmap](00-roadmap.md) để tránh triển khai trùng các tính năng đã có.
