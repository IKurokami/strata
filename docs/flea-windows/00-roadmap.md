# Roadmap triển khai từng PR sau khi đọc tài liệu

**Tài liệu này không kích hoạt bất kỳ feature nào.** Gợi ý các PR tương lai, mỗi PR có acceptance riêng. P0 = lợi ích cao/khối xây dựng trọng yếu; P1 = mở rộng có kiểm soát; P2 = polish/parity; P3 = dự án độc lập lớn.

## Phần 0 — Baseline và chống làm trùng
- Chạy Windows 10/11 interactive smoke có tập fixture địa phương: tab + Space + Columns + Markdown + video + DOCX + XLSX + 3D + archive + filename/content search + SFTP/UNC. Lưu evidence ngoài source repo theo AGENTS.md.
- Ghi danh mục MIME/format/runtime đang đóng gói; kiểm tra từng tính năng có khác biệt portable vs installed; xác nhận preview full-screen hiện hữu trước khi đổi shortcut.
- Ghi file identity, cancellation, event, undo và security threat model theo [architecture](00-cross-cutting-architecture.md).
- Các mục hiện có chỉ sửa khi E2E chỉ ra lỗi (01/02/03/05/08/09/10/15).

## P0 — Missing everyday workflows (implementation PRs nhỏ)
1. [04 Dual pane](04-dual-pane.md): BrowserSession #2 và focus/target model → cross-pane copy/move.
2. [12 Git markers](12-git-markers.md): read-only one-shot batched status per repo; optional setting và cache.
3. [13 Bulk rename](13-bulk-rename.md): dry-run plan, validation, collisions, undo; không phụ thuộc Git/cloud.
4. [14 Compare text files](14-compare-files.md): safe bounded pair diff; có thể tái dùng selector của dual pane sau đó nhưng không bắt buộc.

Lý do: UI/operation core chung; không cần thêm runtime nặng hoặc network/printing. Tất cả phải không phá Windows clipboard, old tabs, undo, chooser, preview.

## P1 — Format/service expansion (split PRs, avoid monolith)
5. [17 Conversion](17-conversion.md) + [18 Combine](18-combine-images.md): dùng chung TransformJob, staged output, extension policy; giữ wizard chooser PNG hiện có.
6. [07 Office](07-office-preview.md) + [06 3D](06-model-3d.md): mở thêm coverage qua sandboxed helpers, optional LibreOffice for fidelity; không ép runtime ngoài nếu không đóng gói an toàn.
7. [16 Printing](16-printing.md): Windows spooler, print preview and explicit consent; feature detect supported formats.
8. [20 Remote SMB/SFTP](20-smb-sftp.md): qualify UNC and SFTP Windows provider; xử lý offline/cancel/security trước khi phát triển cloud.
9. [21 Plugin platform](21-plugin-platform.md): **dùng lại** file provider/custom action, schema/capability versioning, signed/verified install optional; không break manifests cũ.
10. [19 Send to cloud](19-send-to-cloud.md): built-in send target API và consent; cần nền provider/transport ổn định.
11. [11 Content search](11-content-search.md): chỉ phát triển fallback live search khi index opt-in không đáp ứng; tránh hai loại index cạnh tranh.

## P2/P3
- P2 [01/02 Quick Look and preview](01-quick-look.md), [03 Miller](03-miller-columns.md), [05 tabs](05-tabs.md), [08 Markdown](08-rendered-markdown.md), [09 video](09-video-playback.md), [10 recursive search](10-recursive-search.md), [15 archives](15-archives.md): preserve contract, fix proofed gaps only.
- P2 [23 Vim keymap](23-vim-keybindings.md): opt-in profile 10xer; default của Windows giữ nguyên vì có input fields/accessibility.
- P3 [22 TUI](22-terminal-ui.md): separate Rust console frontend once shared non-GTK state/command API is stable.

## Dependency graph
```text
Existing services / bounded jobs / undo / native path rules
 ├── 04 dual-pane ─┐
 ├── 13 bulk rename┼── reusable operation/selection planning
 ├── 12 Git status └── 14 compare (can also be standalone)
 ├── 17 convert ─────── 18 combine
 ├── 06 model / 07 office ── 16 print (optional PDF conversion)
 └── 20 SMB/SFTP ── 21 provider platform ── 19 cloud send
     11 content search independent, only if gap proven
     22 TUI depends on clean shared core, not on GUI rendering
```

## Gates mỗi PR sau này
- **Functionality:** native implementation + documented fallback + keyboard/mouse, file chooser unaffected.
- **Correctness:** deterministic unit/property tests; idempotency or compensating action, no overwrite without chosen policy, no lost source on failure.
- **Security:** per-feature reviewed trust boundary, no arbitrary shell strings, account/path redaction, malformed file tests where relevant.
- **Performance:** own measured first-result/UI blocking/memory profile on Windows; no unbounded directory walks or runaway process.
- **Platform:** NTFS and at least one network/unsupported capability case. Windows interactive GUI smoke required before calling it shipped.
- **Release:** dependencies, optional runtime size, licenses, updater and rollback evaluated; 11-locale coverage where UI text changes.

## Review questions / explicit decisions
1. Giữ default Windows keybinding và thêm Vim profile? Đề xuất: có.
2. Office fidelity có chấp nhận LibreOffice optional cài ngoài? Đề xuất: optional, native hiện tại là mặc định.
3. Cloud gửi qua provider đáng tin hay OAuth API built-in? Đề xuất: provider-scoped trước; sau đó đánh giá từng đích.
4. Tags lưu DB người dùng hay Windows ADS? Đề xuất: local DB + opt-in explicit export; không ghi xattr Linux.
5. TUI cùng binary hay separate executable? Đề xuất: executable riêng chia sẻ core, P3.
6. Windows preview helper có isolation đủ mạnh ngoài Job Object chưa? Cần security review trước mở thêm decoder/parser.

**Định nghĩa Done cho nghiên cứu:** file riêng cho 23 feature, trạng thái + source + Windows plan + MVP + acceptance; **không tuyên bố feature đã hoàn tất**.
