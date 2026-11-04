# 09 — Video playback trong preview

**Trạng thái Strata (source review): ĐÃ CÓ / DUY TRÌ** · **Ưu tiên: P2** · **Design-only — chưa có implementation/Windows GUI acceptance mới**.

## Flea thực sự triển khai thế nào?
Flea sử dụng libmpv qua helper/controller riêng, GPUI vẽ frame/controls và lifecycle. Không suy ra Strata phải thay GStreamer bằng mpv.

### Bằng chứng Flea ghim commit 73f3c239
- [mpv.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/crates/flea-gui/src/g2/pv3/helper/mpv.rs)
- [player.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/crates/flea-gui/src/g2/pv3/player.rs)
- [lifecycle_tests.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/crates/flea-gui/src/g2/pv3/tests/lifecycle_tests.rs)

## Hiện trạng và khoảng trống của Strata Windows
Strata preview đã play video/audio và có scrub/storyboard với GStreamer/FFmpeg runtime. Tính năng không thiếu, cần đo Windows codec/backend và cache eviction.

### Bằng chứng Strata ghim commit 35b24d9a
- [README.md](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/README.md)
- [video.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/ui/preview/video.rs)
- [storyboard.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/media/storyboard.rs)
- [media_windows.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/sandbox/media_windows.rs)
- [media.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/ui/media.rs)

## Thiết kế Windows-native / tích hợp vào code hiện tại
Giữ existing media session; event Play/Pause/Seek/Stop gắn PreviewRequestId, decoder frames gắn media generation; hủy seek khi focus đổi; là một VideoSession active ưu tiên; controls keyboard không chồng Ctrl+... file actions.

## Chia lát triển khai
Chỉ làm quality gates: video orientation/HDR/rotation fallback, large files, seek timeout và accessibility; optional subtitle track nếu nhu cầu.

## Security, correctness, performance, cancel
Locked media file, corrupted MP4/MKV, HW decode GPU driver failure, helper zombie sau close, audio tiếp tục sau tab inactive. Avoid revealing input file to untrusted extensions.

## Tiêu chí nghiệm thu trước merge implementation
Play pause seek preview ổn định, đổi sang file khác stop tức thời, no zombie FFmpeg/decoder, unsupported codec notice thay freeze, keyboard search focus vẫn đúng.

## Quy ước chung
Áp dụng [cross-cutting architecture](00-cross-cutting-architecture.md): GTK không làm blocking I/O; mọi request có generation và hủy; log không chứa path/secret; dùng code Windows thay cho Linux-only APIs; kiểm thử native Windows và negative cases trước khi tuyên bố shipped. Đọc [roadmap](00-roadmap.md) để tránh triển khai trùng các tính năng đã có.
