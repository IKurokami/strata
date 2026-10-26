# Archive extraction

The context menu groups **Extract here**, **Extract to archive folder**, and
**Extract to…** under **Extract**. These commands preserve the original archive
and retain their existing conflict and password behavior.

## Space-saving extraction on Windows

**Extract → Extract and reclaim space…** offers an explicitly confirmed,
destructive mode for single-volume ZIP, 7z, and TAR files. It uses
`hatchzip-core` 0.1.4, rather than implementing a second truncation protocol.
The original archive is progressively consumed, and is removed on completion.
Files are written into the archive's sibling folder without overwriting existing
files. Regular files and directories are supported; archives containing links
or paths through Windows reparse points must use ordinary extraction. RAR,
compressed TAR and split volumes continue to use ordinary extraction.

The engine processes independently decodable units from the end. A 7z solid
block cannot be reclaimed until the entire block has been written. Available
space is checked before starting, and a 256 MiB reserve is maintained. It does
not promise extraction with zero free space. Output is flushed and recovery
state committed before consumed archive bytes are removed.

Cancellation pauses the extraction. Preserve the output folder, remaining
archive and adjacent `.hatchzip` / `.hatchzip-tail` recovery files. Choose the
same command on the remaining archive or its `.hatchzip` journal to resume.
Do not move, edit, replace or delete these files while extraction is unfinished.
Password-protected archives ask for the password again; it is not stored in the
journal. Recovery is restricted to the same archive and its default output
folder. Journals recording another output location, multiple volumes, link
creation or an interrupted restore are refused. Strata does not expose HatchZip's
archive-restoration command.

Upstream protocol and limitations: [HatchZip: How it works](https://github.com/xevrion/hatchzip/blob/d6a4126435cd6a1ed2d158c138d5afe25d131406/docs/how-it-works.md).

## Additional Windows commands

The embedded Windows menu hides canonical Shell verbs already supplied by
Strata: open/explore, open with, copy, cut, delete, rename, properties, copy as
path and ordinary extraction. It preserves unknown verbs, third-party
commands and specialized actions such as running as administrator. Filtering
uses canonical identifiers rather than translated labels. Empty submenus and
redundant separators are removed without renumbering native command IDs.
