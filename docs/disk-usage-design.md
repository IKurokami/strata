# Disk Usage and folder-specific view modes

This is the implementation contract for a proposed fourth browser mode. It does
not describe a feature already shipped. The existing Columns, Icons and List
views remain the navigation and file-operation owners; Disk Usage adds a bounded
treemap renderer and a shared background size index.

## View resolution and persistence

For a normal browser, resolve the exact current folder's explicit mode, falling
back to Columns. Never inherit a parent's mode or the last mode used in another
folder, tab or window. Resolve on navigation before presenting the content;
startup, history restoration and preference notifications are read-only paths.

Only an explicit user view command writes an override. Selecting Columns removes
that field while preserving sort and icon-size fields. Icons, List and Disk Usage
are explicit overrides. Changing an override notifies other open views of that
same folder without changing their selection or moving keyboard focus.

The implementation must separate command dispatch from state application:

- `window::apply_browser_mode` currently applies a mode and writes the global
  preference. Replace the normal-browser command path with exact-folder storage.
- `BrowserView::set_view_mode` applies resolved state and must never persist it.
- `BrowserView::connect_view_mode_changed` observes the GTK stack, not user intent;
  its notification must not become a persistence trigger.
- `browser/preferences.rs` currently binds the global `browser_mode`. Replace that
  normal-browser binding with location plus mode-revision resolution. Keep the
  binding immediate and live, including before Settings is opened.
- The chooser has a separate policy: default Columns, no Disk Usage and no reads
  or writes of normal-browser mode overrides. Shared view-command helpers need
  an explicit caller policy, rather than inferring one from visible widgets.

Extend `FolderView`, `StoredFolder` and field-tolerant parsing in
`ui/preferences/folder_views.rs` with an optional serialized mode. Keep the
version-1 additive format: older readers skip the new field. An unknown mode
discards only that field, retaining valid sort and icon size. Merge-on-save,
external-file notifications, subtree relocation, trash restoration and atomic
writes remain owned by `folder_view_settings.rs` and `FolderViews`.

Entries with an explicit mode are durable. `evict_to` must count and evict only
ephemeral entries; subtracting a total count containing protected entries would
still discard too much sort/icon state. Clearing the usage index never clears
folder preferences. A deliberate reset of folder customizations may clear both.

Mode persistence is mandatory for the normal browser and is independent of the
existing optional remembered-sort/icon-size preference. Adjust that control's
description in all catalogs so the distinction is visible. The legacy global
`browser_mode` remains readable for compatibility and chooser policy, but does
not seed unspecified normal folders after migration. Document the switch to
Columns for previously unconfigured folders when the feature ships.

Use existing `FolderKey::Volume { uuid, relative }` where a removable identity is
available, otherwise a normalized native path. Current `key_for_location`
deliberately excludes network locations; do not silently claim remote override
support. A future remote key must remove credentials and distinguish provider
identity. Known successful rename/move mappings relocate overrides; external
renames without a trustworthy identity mapping must not guess.

## Service and storage boundary

Add a process-wide `services::disk_usage` service, independent of widget lifetime.
Its owned background workers manage scanning, persistence and reconciliation.
GTK receives immutable, bounded snapshots through a coalesced notification lane.
No recursive metadata access, database read, cache deserialization or lock-held
filesystem operation runs on the GTK context.

The service contract includes these distinct values:

| Value | Meaning |
| --- | --- |
| Root key | Volume/provider identity plus location; never just a drive letter |
| Node identity | Filesystem identity plus incarnation where available, with a path fallback |
| Navigation token | Subscriber's current location and epoch; rejects stale UI deliveries |
| Index generation | Committed contents revision of a root/subtree |
| Cache epoch | Changes on cache clear; prevents old workers resurrecting deleted data |
| Coverage | Counts, error categories, limits and excluded boundaries, independent of bytes |

`LocationIdentity` currently contains 64-bit device/inode fields. Do not truncate
a Windows 128-bit file ID to fit it: retain a platform-native identity inside the
usage index until a shared identity extension is designed. On unsupported filesystems,
path identities require revalidation and must not claim rename continuity.

Snapshots expose direct children, logical bytes, optional allocated bytes,
descendant file counts, coverage, generation and state. States are Exact,
Partial, Stale and Unavailable. An empty Exact folder has zero bytes; a denied
folder has unknown bytes, not zero. Stale snapshots retain their previous known
total and timestamp while reconciliation runs. Partial percentages use the known
scanned total and are explicitly labeled accordingly.

Reuse the existing SQLite dependency for a separate, versioned usage database,
not the Smart Search database. A single background writer owns WAL transactions;
indexed reads also run off GTK. Store parent relationships, identities, measured
metadata, aggregates, root coverage and completion/checkpoint markers. Publish
only committed generations. Interrupted scans preserve the previous committed
snapshot and an explicitly partial checkpoint. Corrupt/unsupported caches are
quarantined and rebuilt lazily; errors must not affect source files or settings.

Use a 64 MiB soft RAM-cache budget and a 512 MiB disk-cache budget initially,
measured by actual retained data. Eviction affects usage data only. These are
starting resource limits, not measured performance claims. Persistent child
queries are bounded and aggregate the tail; an enormous directory must not
materialize a million children on the GTK thread.

## Scanning and reconciliation

Cold entry shows the browser shell immediately. A background direct-child batch
can produce a Partial snapshot before descendant totals are known. An iterative
queue measures metadata without following symlinks, junctions, mount crossings
or cloud placeholders. Recognized remote locations require explicit scan consent
and their own I/O budget; selecting Disk Usage does not authorize a network crawl.
Unreadable children and unknown filesystem boundaries make coverage Partial.

Start with one worker per rotational/unknown volume and at most two per known SSD,
four outstanding scans globally. Metadata buffers and handles are bounded. A
root request deduplicates with existing work: overlapping parent/child scans share
node records and measurements. Changing subscribers reprioritizes work; leaving
a view drops UI delivery interest without deleting measured data.

Store parents before children, then aggregate bottom-up in O(N). Updates apply
deltas up known ancestors; a rename invalidates/reconciles both old and new parent
chains. Cross-volume moves are delete-plus-insert. Use checked arithmetic;
overflow sets a coverage flag and cannot produce an Exact total. Logical bytes
are path-based: hard-linked paths can contribute repeatedly. Allocated totals
and reclaimable-space estimates require separate identity/deduplication semantics;
do not present the logical sum as guaranteed reclaimable space.

Use public native batched enumeration through platform adapters with portable
fallbacks. Benchmark Windows large-fetch enumeration before choosing an optimized
default. Watch only active/recent scopes within a bounded handle budget. Coalesce
changes and reconcile affected subtrees; watcher overflow, journal reset, sleep/
resume uncertainty and disconnected volumes mark affected data Stale. Optional
NTFS USN support is a capability optimization, not a requirement for basic use
or a reason to prompt for elevation.

Warm navigation uses a RAM snapshot or asynchronous indexed read; it does not
restart a recursive scan. Reconcile only on invalidation, stale coverage or an
explicit refresh. Cache clear increments the epoch, cancels old writes, closes
database users and removes usage data on a worker before reopening a fresh index.
Concurrent scans cannot write back under the old epoch.

## Treemap and browser integration

Add `BrowserMode::DiskUsage` throughout `ModeViews`, mode controls, events,
navigation/focus restoration, filtering and shared preferences. This PC remains
a distinct virtual location rather than a recursive usage root. Keep the sidebar,
breadcrumb, preview rules and existing browser history.

Use one native canvas/custom snapshot widget. A pure Rust squarify implementation
consumes a bounded snapshot, sorts positive weights deterministically by size and
stable identity, and groups the remaining entries into an explicit Other result.
Start at 300 tiles maximum, adapting visible labels and minimum area to viewport
and scale. Other opens a bounded list of real entries; it is not a synthetic
filesystem path and cannot itself be renamed, dragged or deleted.

Tile identity and source-entry identity remain separate from transient ordering.
Actions resolve a selected identity against the latest browser/index revision,
then use existing open, preview, clipboard, rename, transfer and deletion dispatch.
Never interpret an old tile's array index as a current file index. Filters change
visible membership without silently changing the meaning of the measured scope
total. Hidden-file exclusions must be stated when they affect totals.

Single click selects and follows existing preview policy. Double click enters a
folder or dispatches the existing file action. Right click selects the action
target without opening its preview. Keyboard commands use the shared bindings
catalog; spatial arrows select the nearest eligible tile. Context menus and
operation confirmation/undo remain shared. Outbound drags use the same provider
and lifecycle described in [Windows file dragging](windows-drag-drop-design.md).

Reuse semantic theme colors, font preferences and the shared icon path. Hover
changes highlight only, without rescanning or relayout. Render retained labels
and avoid CPU layout work in pointer motion. Default transitions use a short
identity-based interpolation; reduced motion, rapid navigation and resize apply
the latest state directly. Interruptions start from the current displayed state,
not a queued old animation.

Expose stable accessible children with names, size, coverage, selection and
activation, and keyboard navigation equivalent to the visual canvas. Validate
the pinned GTK accessible-child APIs before committing to a custom widget; an
inaccessible painted-only canvas is not a completed implementation. Repository
tooltip policy permits tooltips only on icon buttons: show full tile details in
the inspector/status surface and accessible descriptions instead of tile tooltips.

## Validation ownership

Extend the existing folder-preference tests for exact-folder resolution, children
defaulting to Columns, restart, unknown mode with valid sort retained, protected
overrides beyond the LRU limit, mode/sort merge conflicts, relocation and trash
restoration. Extend live browser-preference coverage for startup before Settings,
two consumers of the same folder, view rebuilds and chooser isolation.

Service tests own totals, partial/denied states, overflow, symlink/mount boundaries,
cache corruption, crash checkpoints, duplicate subscribers, stale generations,
cache-clear races and file-operation invalidation. Pure algorithm tests cover
invalid/nonfinite weights, deterministic identity and bounded grouping, not tile
geometry. Repository policy prohibits layout tests; verify placement, resizing,
animation and clipping manually with sanitized captures.

Keep behavioral E2E cases for mode commands, history, selection, operations and
keyboard routes. Performance runs separately measure input-to-frame percentiles,
warm cache reuse, scan duplication, queue depth, handles and RAM on synthetic
small/large trees. Proposed p95 goals are 16 ms selection, 50 ms cached navigation
and 100 ms cold shell response; these are targets until measured on named hardware.
Full Windows/Linux filesystem and installed-artifact checks remain release gates,
not claims implied by unit tests. Localize every new application string in all
eleven catalogs and extend the saved-preferences fixture and i18n regressions.

## References

- [Preference lifecycle](preferences.md), [keyboard shortcuts](keyboard-shortcuts.md)
  and [internationalization](internationalization.md).
- [Strata Files treemap](https://github.com/cristipufu/strata-files/blob/main/Sources/Strata/Views/Treemap.swift)
  provides a design reference for weighted grouping and squarification. Reuse
  architecture ideas; copying implementation requires license/provenance review.
