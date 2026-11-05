# 19 — Send to cloud / share target

**Trạng thái Strata (source review): MỘT PHẦN INFRA** · **Ưu tiên: P1** · **Design-only — chưa có implementation/Windows GUI acceptance mới**.

## Flea thực sự triển khai thế nào?
Flea plugin kinds include send-target, and cloud/network actions can be supplied by installed integrations via versioned JSON-lines, not an obligatory built-in service; GPUI renders host-owned menu and receives provider results.

### Bằng chứng Flea ghim commit 73f3c239
- [PLUGINS.md](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/PLUGINS.md)
- [plugins.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/src/backend/plugins.rs)
- [network_cloud.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/crates/flea-gui/src/window/network_cloud.rs)
- [menu_send.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/crates/flea-gui/src/g2/plugins/menu_send.rs)

## Hiện trạng và khoảng trống của Strata Windows
Strata has Windows external file providers for badges/menu actions, not a unified explicit 'Send to cloud' destination workflow with consent, progress and settled upload outcomes. File providers are trusted executables, not managed OAuth provider accounts.

### Bằng chứng Strata ghim commit 35b24d9a
- [file-providers.md](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/docs/file-providers.md)
- [file_providers_windows.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/services/file_providers_windows.rs)
- [file_providers.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/ui/file_providers.rs)
- [remote_download.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/services/remote_download.rs)

## Thiết kế Windows-native / tích hợp vào code hiện tại
Define SendTarget capability as new provider contract version or separate opt-in extension: id/label/acceptedKinds/estimatedLimits, DryRun per selection, user consent with target/account/display path, UploadJob progress/cancel/retry and result URL. Use authorized provider process with versioned response IDs; no keys passed in argv/log, passwords via OS Credential Manager where integration requires. Distinguish 'queued', 'accepted by provider', 'uploaded verified'. Prefer local sync-folder action first; cloud API direct only after threat model.

## Chia lát triển khai
MVP: Send to local configured sync folder (OneDrive/Dropbox etc only if user chooses existing local folder) with safe copy, confirm and open destination; optional trusted provider send-target subsequent, no guessed accounts/automatic cloud access.

## Security, correctness, performance, cancel
Accidental upload/private documents, duplicate/partial remote objects, offline/auth-expired, credential leakage, file mutated during stream, provider hangs or lies about completion, file-sharing permissions and expiry. Explicit host/path review and cancel semantics.

## Tiêu chí nghiệm thu trước merge implementation
Nothing transmitted without click+confirmation; fixtures offline/401/429/partial-upload and cancel; source preserved, progress truthful, identity/target displayed, no raw file path/token in logs or exception traces.

## Quy ước chung
Áp dụng [cross-cutting architecture](00-cross-cutting-architecture.md): GTK không làm blocking I/O; mọi request có generation và hủy; log không chứa path/secret; dùng code Windows thay cho Linux-only APIs; kiểm thử native Windows và negative cases trước khi tuyên bố shipped. Đọc [roadmap](00-roadmap.md) để tránh triển khai trùng các tính năng đã có.
