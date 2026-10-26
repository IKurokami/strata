# Preview memory and background tools

Settings → General → Performance → **Clear preview cache** releases unused
preview data and in-memory thumbnails. The Windows tray menu provides the same
action. Open previews keep their own content. Windows also removes regular
hash-named files from Strata's private thumbnail cache on a worker thread;
settings, original files, nested folders and links are preserved. The Linux
Freedesktop disk thumbnail bucket is shared with other applications and is
left intact. Ongoing previews can generate new cache entries after clearing.

Large text/code previews and unusually long lines use virtual source rows even
when a syntax language is recognized. This includes JSON, XML, JavaScript and
other code formats. Replacing a source preview cancels pending buffer inserts,
disables the previous syntax engine, and releases its text and undo history.
Rendered code blocks use the same syntax budget. Normal small files retain
syntax highlighting. Interactive reads remain bounded to the first 1 MiB.

New-tab bursts preserve every requested tab, but defer loading an unvisited tab
until it remains visible. Sidebar discovery also waits for a stable mapped tab.
Session snapshots are coalesced and written serially on a worker thread, with
a flush at window close/application shutdown.

The shared preview cache has a 128 MiB retained-data budget. Accounting includes
image buffers, document text and spans, archive trees, and rendered-page buffers,
including reserved buffer capacity. Entries larger than the budget are not kept.
Open previews retain their own content independently of cache eviction.

An idle cache expires after 90 seconds without a read or insertion, checked every
30 seconds on the main loop. Clearing also releases the cache's backing
allocations. Active previews are not interrupted.
Providers share one maintenance timer on their thread. Dropping the last provider
cancels that source and releases the unused cache immediately, so a callback
cannot remain behind after its preview owners have gone away.

On Windows, sandboxed decoder workers already retire after an idle period. They
also retire after returning a response if they have handled 128 requests or their
private job reports a peak process memory usage of at least 256 MiB. A failed
memory query also retires the worker. The next request starts a fresh worker;
active decoding is not terminated to enforce these reuse thresholds.

These policies bound application-owned reuse. They do not establish a total RAM
limit or claim to fix all GTK, GStreamer, graphics-driver, or allocator retention.

Background media probes, bounded helper commands and Windows shell helpers use
the Windows `CREATE_NO_WINDOW` flag. Output and exit status remain available to
the application. Explicitly launched terminals and the application's **Show
console** setting retain their normal behavior.
