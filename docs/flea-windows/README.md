# Flea → Strata for Windows: feature design index

> **Scope:** research and proposed design, not implementation. Base revisions: Flea [73f3c23](https://github.com/thisisgm/flea/tree/73f3c23971106f6d47f78bd758f783af3e5b5c78) (2026-11-03) and Strata [35b24d9](https://github.com/IKurokami/strata/tree/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2) (2026-11-04). Language: Vietnamese; code symbols and Windows API names stay in English.

## Đọc tài liệu theo thứ tự
1. [Research notes](00-flea-source-research.md): cơ chế thực thi của Flea hiện hành, dẫn mã nguồn chính xác.
2. [Cross-cutting architecture](00-cross-cutting-architecture.md): mô hình ứng dụng, security boundary, hủy công việc và hợp đồng Windows.
3. [Roadmap](00-roadmap.md): thứ tự triển khai, phụ thuộc, thử nghiệm và quyết định chưa chốt.
4. Một file **riêng cho từng tính năng** ở bảng dưới. Tất cả là RFC / implementation proposal; **không tuyên bố đã chạy hoặc kiểm thử Windows thực tế**.

Trạng thái = đánh giá dựa trên nguồn hiện có, **không phải** xác nhận nghiệm thu GUI. `Có` nghĩa là đã có đường đi cụ thể; `Một phần` nghĩa là thiếu phạm vi/UX/parity; `Thiếu` nghĩa là không tìm được chức năng tích hợp native tương đương ở revision khảo sát. Dễ nhầm nhất: split preview khác dual-pane; custom actions khác plugin runtime; image-to-PNG chooser khác batch converter.

| # | Flea feature | Strata hiện trạng | Ưu tiên | Thiết kế |
|---|---|---|---|---|
| 01 | Quick Look (Space, full window) | Một phần (Space preview, full-window mode cần bổ sung/xác minh) | P2 | [01](01-quick-look.md) |
| 02 | Preview pane | Có; hoàn thiện lifecycle | P2 | [02](02-preview-pane.md) |
| 03 | Miller columns | Có | P2 | [03](03-miller-columns.md) |
| 04 | Dual pane / split view | Thiếu dual-browser thật | P0 | [04](04-dual-pane.md) |
| 05 | Tabs | Có | P2 | [05](05-tabs.md) |
| 06 | 3D model preview | Một phần (STL/3MF/FreeCAD) | P1 | [06](06-model-3d.md) |
| 07 | Office preview | Một phần (DOCX/spreadsheet subset) | P1 | [07](07-office-preview.md) |
| 08 | Rendered Markdown | Có | P2 | [08](08-rendered-markdown.md) |
| 09 | Video in preview | Có; mở rộng tùy nhu cầu | P2 | [09](09-video-playback.md) |
| 10 | Recursive search as you type | Có | P2 | [10](10-recursive-search.md) |
| 11 | File content search | Có, có điều kiện opt-in index | P1 | [11](11-content-search.md) |
| 12 | Git status markers | Thiếu built-in; providers có thể bổ sung metadata | P0 | [12](12-git-markers.md) |
| 13 | Bulk rename | Thiếu dialog/native batch | P0 | [13](13-bulk-rename.md) |
| 14 | Compare files | Thiếu native viewer | P0 | [14](14-compare-files.md) |
| 15 | Create/extract archives | Có; gia cố parity | P2 | [15](15-archives.md) |
| 16 | Print | Thiếu native workflow | P1 | [16](16-printing.md) |
| 17 | Convert formats | Một phần (image→PNG chooser; scripts) | P1 | [17](17-conversion.md) |
| 18 | Combine images | Thiếu | P1 | [18](18-combine-images.md) |
| 19 | Send to cloud | Một phần (provider integration framework) | P1 | [19](19-send-to-cloud.md) |
| 20 | SMB and SFTP | Một phần (UNC/GIO/provider differences) | P1 | [20](20-smb-sftp.md) |
| 21 | Plugins / extensions | Một phần (custom actions and file providers) | P1 | [21](21-plugin-platform.md) |
| 22 | Terminal UI version | Thiếu (có launch terminal, không phải TUI) | P3 | [22](22-terminal-ui.md) |
| 23 | Vim keys (hjkl) by default | Có mode 10xer tùy chọn, không mặc định | P2 | [23](23-vim-keybindings.md) |

## Nguyên tắc review
- Không viết lại tính năng Strata đã làm tốt. Mỗi RFC phải chỉ ra điểm móc vào hệ hiện có và phiên bản MVP nhỏ nhất.
- Ưu tiên **không mất dữ liệu / không đóng băng UI / không tự chạy code không tin cậy** hơn sao chép UI Flea.
- Các số timeout/budget đề xuất là **mục tiêu cần đo**; nếu nguồn ghi budget Flea thì chỉ áp dụng cho Flea.
- Bản thiết kế phải cập nhật cùng implementation, test và tài liệu có sẵn khi code thực tế đổi.
- Không xem Linux bubblewrap, inotify, xattr, CUPS, GVfs, libmpv hay GPUI là API dùng trực tiếp được trên Windows.
