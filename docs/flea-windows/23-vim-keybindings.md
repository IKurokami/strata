# 23 — Vim keys (hjkl) by default

**Trạng thái Strata (source review): CÓ OPTIONAL 10XER, KHÔNG MẶC ĐỊNH** · **Ưu tiên: P2** · **Design-only — chưa có implementation/Windows GUI acceptance mới**.

## Flea thực sự triển khai thế nào?
Flea có keymap/sheet và Vim-style keyboard-first TUI/GUI. Các tổ hợp được định nghĩa trong key table, đồng bộ tài liệu với UI; không hardcode mọi nơi.

### Bằng chứng Flea ghim commit 73f3c239
- [mod.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/src/tui/mod.rs)
- [the-key-table-is-generated.md](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/docs/architecture/the-key-table-is-generated.md)
- [keymap_sheet_activation_tests.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/crates/flea-gui/src/g2/st1/keymap_sheet_activation_tests.rs)

## Hiện trạng và khoảng trống của Strata Windows
Strata 10xer mode dùng Yazi/Vim-style navigation và là option hiện có. User screenshot ghi 'by default' nhưng đây là **quyết định sản phẩm**, không phải bug nếu Windows-oriented default hiện tại khác.

### Bằng chứng Strata ghim commit 35b24d9a
- [keyboard-navigation.md](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/docs/keyboard-navigation.md)
- [keyboard-shortcuts.md](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/docs/keyboard-shortcuts.md)
- [tenxer_mode.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/ui/tenxer_mode.rs)
- [preview.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/ui/keys/bindings/preview.rs)
- [navigation.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/ui/keys/bindings/navigation.rs)

## Thiết kế Windows-native / tích hợp vào code hiện tại
Giữ WindowsProfile mặc định để tương thích Explorer và dialog; cung cấp 'Keyboard profile: Windows / 10xer / Custom (future)' rõ ràng trong Settings. Routing duy nhất ở ui::keys/bindings, priority: modal > text input/IME > preview/media > browser > global. hjkl chỉ ở browser focus, tuyệt đối không ăn phím khi path/search/rename editor đang nhập. 10xer có visual mode hints, shortcut reference dynamic, conflict detector cho custom.

## Chia lát triển khai
MVP là tài liệu/keymap acceptance, không đổi default; sau đó UI chọn profile và onboarding users có option 'Make Vim profile default' per-user. Chỉ đổi default nếu chủ dự án chủ động quyết định breaking UX.

## Security, correctness, performance, cancel
Keyboard layout non-Latin/Vietnamese IME, accessibility, Ctrl+C windows clipboard, text fields receiving hjkl, command chords waiting forever, shortcuts OS reserved; keymap profile persist safely.

## Tiêu chí nghiệm thu trước merge implementation
10xer hjkl navigate, typing in filename/search remains text, IME composition unaffected, tabs/modal Esc precedence, user can restore Windows profile and persisted choice across restart; F1 renders actual labels from binding catalog.

## Quy ước chung
Áp dụng [cross-cutting architecture](00-cross-cutting-architecture.md): GTK không làm blocking I/O; mọi request có generation và hủy; log không chứa path/secret; dùng code Windows thay cho Linux-only APIs; kiểm thử native Windows và negative cases trước khi tuyên bố shipped. Đọc [roadmap](00-roadmap.md) để tránh triển khai trùng các tính năng đã có.
