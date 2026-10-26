# Preview memory and background tools

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
