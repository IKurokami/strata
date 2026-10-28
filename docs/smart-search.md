# Local content search

Global search retains its filename worker and adds independent FTS5 and semantic
workers. Folder history and inline filename filtering keep their existing roles.
Documents and queries are not uploaded. Logs contain generations, durations and
counts rather than queries or document contents.

Add roots in **Settings → Smart Search → Indexed folders**. No roots are selected
by default. Code/configuration indexing requires a separate opt-in and database.
Secrets, build/vendor caches, symlinks, `CACHEDIR.TAG` directories and Windows
cloud placeholders are excluded. Removing a root removes its index records;
unavailable roots are retained until a complete scan can reconcile them.

Semantic search defaults to on, including when older settings lack the field.
Startup and opening search do not download or load models. **Download models**
explicitly downloads two pinned Q8_0 GGUF files (864,676,480 bytes), resumes partial
downloads and verifies exact sizes and SHA-256. Filename and keyword search work
without model weights. Disabling Smart Search cancels semantic work and releases
inference/vector caches while filename and keyword search continue. Indexing can
be paused and battery indexing defaults to off. Rebuild/delete actions require
confirmation and keep source files and downloaded models. Delete also pauses
indexing until resumed.

The virtualized result list supports name/content/semantic modes, file-type and
indexed-folder scopes. Selection follows file/segment identity through later
phases; late keyword results cannot replace semantic results. Preview starts on
selection and releases its decoder and space when disabled. File revisions and
typed locators travel to the existing preview provider. PDF pages are zero-based
internally and use the existing glyph overlay. Text offsets count Unicode
characters; code lines and worksheet cells are one-based. Media results identify
approximate windows rather than word-level transcripts. Seeking waits for the
same media stream to be ready and cancels after user interaction or a new request.
Supported external readers/players receive page/line/time arguments without a
shell. Other applications open normally. Slide and worksheet results retain an
explicit location/excerpt and display a notice if the preview cannot jump there.

Budgets: 200,000 discovered files, 512 MiB index/WAL data, 80,000 resident FP16
vectors across databases, 256 query embeddings and five-minute idle eviction.
Plain text/Office XML extraction is capped at 4 MiB, responses at 1 MiB, and
resumable document cursors at 1,024 pages/sheets. Unsupported or oversized inputs
are skipped. Visual media samples use 10-second intervals; audio uses 30-second
windows with independent 25-second hops. The indexer yields to interactive work.

Fast adjacent regressions cover SQLite, ranking, protocol validation, cancellation,
settings and locator consumers; run them through `scripts/test-headless.py` on a
private display. They do not download models. Real-model retrieval quality and
click-to-painted-frame targets need measured fixtures and are not established by
unit tests. See [runtime provenance](../packaging/smart-search/README.md).
