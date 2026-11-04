# 12 — Git status markers

**Trạng thái Strata (source review): THIẾU BUILT-IN** · **Ưu tiên: P0** · **Design-only — chưa có implementation/Windows GUI acceptance mới**.

## Flea thực sự triển khai thế nào?
Flea tìm .git repo trong cây mà không chạy Git, sau đó read-only git porcelain v2 snapshot với worker ordered, epoch invalidation và bounded output. Status snapshot map vào rows hiện hữu, **không spawn Git per row**.

### Bằng chứng Flea ghim commit 73f3c239
- [gitprobe.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/src/backend/gitprobe.rs)
- [gitstatus.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/src/backend/gitstatus.rs)
- [gitreq.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/src/backend/gitreq.rs)
- [gitbudget.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/src/backend/gitbudget.rs)

## Hiện trạng và khoảng trống của Strata Windows
Strata đã có provider protocol để bên thứ ba gắn badge, nhưng không thấy Git marker built-in có trải nghiệm zero-setup như Flea; cần xác thực điều này bằng GUI. Không nên bắt người dùng viết provider riêng để có icon Git cơ bản.

### Bằng chứng Strata ghim commit 35b24d9a
- [file-providers.md](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/docs/file-providers.md)
- [file_providers_windows.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/services/file_providers_windows.rs)
- [file_providers.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/ui/file_providers.rs)
- [entry.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/ui/browser/entry.rs)

## Thiết kế Windows-native / tích hợp vào code hiện tại
GitStatusService dùng discovery/cached RepositoryId trong directory ancestor; read git.exe status --porcelain=v2 -z --branch --untracked-files=normal (argv list only) trong worker; parse NUL records, rename/from paths, ignored/untracked/modified/conflicted; cache immutable per-repo snapshot (worktree state + index mtime + TTL) và decorate only visible items. Respect nested repositories/submodules/worktrees and Git executable trust/availability.

## Chia lát triển khai
MVP: local folder in Git repo shows changed/untracked/conflict markers; opt-in setting per UI/battery mode; manual Refresh. Later directory aggregate status, workspace watch with debounce and provider override policy.

## Security, correctness, performance, cancel
Git config aliases/hooks/filter process behavior; launch Git with safe config/command and reviewed environment, never trust arbitrary repo hooks; repository discover symlink-safe, no prompt/network, output/timeout cap, stale snapshot discarded, graceful if git missing.

## Tiêu chí nghiệm thu trước merge implementation
Fixture with staged/unstaged/untracked/conflict/rename/submodule; one worker per repo not per file, no Git on non-repo, no UI jank, no stale badge after move/refresh, UNC/network opt-in or bounded fallback.

## Quy ước chung
Áp dụng [cross-cutting architecture](00-cross-cutting-architecture.md): GTK không làm blocking I/O; mọi request có generation và hủy; log không chứa path/secret; dùng code Windows thay cho Linux-only APIs; kiểm thử native Windows và negative cases trước khi tuyên bố shipped. Đọc [roadmap](00-roadmap.md) để tránh triển khai trùng các tính năng đã có.
