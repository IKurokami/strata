# Camera RAW previews

**Settings → General → Performance → Fast RAW preview** defaults to on. FlashRaw
0.1.1 runs inside the existing sandbox helper only for validated RAW containers.
Ordinary JPEG/PNG/WebP and media keep their existing decoders.

The helper selects the smallest embedded JPEG/RGB preview meeting the requested
edge, or the largest undersized preview. It applies per-preview orientation once
and converts embedded JPEG ICC profiles to sRGB once. List thumbnails never
request sensor development. Selected previews prefer embedded pixels and use
bounded half-size CPU development when needed. **Develop RAW** requests higher
detail and retains the current picture until replacement; failure keeps that
picture. GPU initialization is not on this path. Disabling the option routes RAW
requests to the legacy decoder.

Input uses an owned bounded snapshot, avoiding mmap faults if a source is
truncated. Reads are capped at 256 MiB, embedded previews at 32 MiB/16 MP, and
sensor development at 32 MP. Unsupported/oversized inputs retain legacy fallback.
New thumbnails cross the existing worker transport as `STRGBA01`, little-endian
width/height and exactly width × height × 4 tightly packed RGBA8 bytes. Validation
precedes GTK memory-texture creation. Legacy PNG stays accepted; persistent PNG
encoding runs on the bounded cache executor after display. Memory cache keys
include revision, decoder choice and quality where applicable.

Trace records distinguish embedded/CPU providers and decode durations. Adjacent
regressions cover candidates, malformed containers, JPEG row strides, transport
bounds and development lifecycle. Real camera coverage and first-painted-frame
performance require measured files; decoder timing alone is not a UI benchmark.
