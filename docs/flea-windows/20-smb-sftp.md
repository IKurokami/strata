# 20 — SMB and SFTP shares

**Trạng thái Strata (source review): MỘT PHẦN / WINDOWS VERIFY** · **Ưu tiên: P1** · **Design-only — chưa có implementation/Windows GUI acceptance mới**.

## Flea thực sự triển khai thế nào?
Flea dùng địa chỉ network UI và Linux GIO/GVfs; FUSE remote có listing một pass qua gio list vì per-item stat cực chậm. Kết nối SFTP/SMB trên Linux phụ thuộc daemon/mount; không bê FUSE/GVfs sang Windows.

### Bằng chứng Flea ghim commit 73f3c239
- [networkaddress.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/src/backend/networkaddress.rs)
- [network.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/crates/flea-gui/src/window/network.rs)
- [network_path.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/crates/flea-gui/src/window/network_path.rs)
- [two-phase-listing.md](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/docs/architecture/two-phase-listing.md)

## Hiện trạng và khoảng trống của Strata Windows
Strata Windows dùng UNC native path cho SMB và đã có Windows provider transport; tài liệu SFTP cũ mô tả GIO/GVfs Linux, cần chạy thực tế với provider Windows trước khi tuyên bố full parity. UNC offline/SMB file locks/host key chưa có bằng chứng acceptance trong nghiên cứu này.

### Bằng chứng Strata ghim commit 35b24d9a
- [remote-sftp.md](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/docs/remote-sftp.md)
- [windows-feature-compatibility.md](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/docs/windows-feature-compatibility.md)
- [file_providers_windows.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/services/file_providers_windows.rs)
- [transport_windows.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/services/file_providers/transport_windows.rs)
- [native_fs.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/services/native_fs.rs)

## Thiết kế Windows-native / tích hợp vào code hiện tại
Transport abstraction: FileLocation {LocalNative, UNC, SftpUri} with capabilities {list,streamRead,write,atomicRename,trash,fileId,watch}. UNC handled native Win32 with latency/cancel and credential prompt by OS; do not hardcode SMB user/password. SFTP via explicitly configured Windows-compatible SSH transport/provider with known_hosts host-key verification, OS credential store/agent, typed URI without passwords, bounded queues/retries; prefer streaming, avoid temp full download where possible.

## Chia lát triển khai
MVP 1: UNC browse/copy/move/read-only/offline matrix including mapped drives; protect GTK from blocking calls. MVP 2: Windows SFTP listing/download/upload in disposable OpenSSH fixture, host-key first-use user approval; no silent trust-on-first-use.

## Security, correctness, performance, cancel
UNC rename/copy semantics, server-side capability mismatch, infinite DNS wait, partial remote writes, malicious server names, links to outside share, reconnect auth, lost network mid-transfer, no permanent-delete fallback for missing remote trash.

## Tiêu chí nghiệm thu trước merge implementation
SMB UNC and mapped share open, disconnect recovers, no UI stall; SFTP nondefault port/password/key/host-key refusal and retry, no credentials in URI/log, interrupted 1GiB transfer preserves source and can safely report partial target.

## Quy ước chung
Áp dụng [cross-cutting architecture](00-cross-cutting-architecture.md): GTK không làm blocking I/O; mọi request có generation và hủy; log không chứa path/secret; dùng code Windows thay cho Linux-only APIs; kiểm thử native Windows và negative cases trước khi tuyên bố shipped. Đọc [roadmap](00-roadmap.md) để tránh triển khai trùng các tính năng đã có.
