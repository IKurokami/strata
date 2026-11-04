# 21 — Plugins / extensions

**Trạng thái Strata (source review): MỘT PHẦN** · **Ưu tiên: P1** · **Design-only — chưa có implementation/Windows GUI acceptance mới**.

## Flea thực sự triển khai thế nào?
Flea 0.4.0 plugin manifest schemaVersion 1 với contribution kinds: menu-action, rail-section, place, previewer, send-target, shelf; process supervised JSON-lines, first-party UI controls/settings; previewer có jail riêng, permission declarations và startup handshake.

### Bằng chứng Flea ghim commit 73f3c239
- [PLUGINS.md](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/PLUGINS.md)
- [plugin-protocol.json](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/docs/plugin-protocol.json)
- [pluginruntime.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/src/backend/pluginruntime.rs)
- [pluginhost.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/src/backend/pluginhost.rs)
- [pluginpreview_sandbox.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/src/backend/pluginpreview_sandbox.rs)
- [host.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/crates/flea-gui/src/g2/plugins/host.rs)

## Hiện trạng và khoảng trống của Strata Windows
Strata đã có custom actions (user-trusted scripts) và external file provider protocol v1 (badges/actions), nhưng không có 1 extension package manager thống nhất cho previewer/sidebar/send-target. Không được xóa API provider hiện tại để copy Flea manifest.

### Bằng chứng Strata ghim commit 35b24d9a
- [custom-actions.md](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/docs/custom-actions.md)
- [file-providers.md](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/docs/file-providers.md)
- [file_providers_windows.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/services/file_providers_windows.rs)
- [settings.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/ui/settings.rs)
- [architecture.md](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/docs/architecture.md)

## Thiết kế Windows-native / tích hợp vào code hiện tại
Giữ 2 API tương thích; thêm ExtensionRegistry v1 với explicit typed contributions (menu/actions, sidebar-place, file-decorations; later previewer/send-target). Manifest parsed from per-user APPDATA, allowlist API/schema, canonical non-reparse files and owner ACL, per-extension lifecycle and no dynamic DLL. Versioned bounded JSON-lines, request IDs/gen, timeout/output cap and per-provider job supervision; UI owns all settings and menu controls. Trusted action vs untrusted previewer MUST have distinct permission/security levels; untrusted code cannot be justified as safe solely by Job Object.

## Chia lát triển khai
MVP: settings catalog install-from-folder/enable/disable/remove with warnings, backward compatibility adapters to existing action/provider manifests; harden message validation and bad extension recovery. Next sidebar place, preview contributions with reviewed sandbox, cloud send targets.

## Security, correctness, performance, cancel
Supply-chain malicious plugin, TOCTOU manifest/exe swaps, symlink junction/powershell launch, arbitrary file/read/write, binary crash, path/credentials leak, protocol injection, unbounded UI content. No third-party process in GTK address space; no automatic downloads/updates without consent.

## Tiêu chí nghiệm thu trước merge implementation
Protocol version negotiation/refusal, registry enable-disable persists, invalid path/ACL/reparse rejects, crashing/hung extension leaves core responsive, limits enforced, existing provider/custom actions still work; previewer cannot access arbitrary user directory under security tests.

## Quy ước chung
Áp dụng [cross-cutting architecture](00-cross-cutting-architecture.md): GTK không làm blocking I/O; mọi request có generation và hủy; log không chứa path/secret; dùng code Windows thay cho Linux-only APIs; kiểm thử native Windows và negative cases trước khi tuyên bố shipped. Đọc [roadmap](00-roadmap.md) để tránh triển khai trùng các tính năng đã có.
