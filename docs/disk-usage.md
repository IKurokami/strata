# Disk Usage

Select **Appearance → Disk Usage** or press **Ctrl+4** to show the current native
folder as a treemap. Area represents measured logical bytes in that folder,
including hidden files. It does not represent whole-drive capacity or guaranteed
reclaimable space. Hard-linked paths can each contribute to the logical sum.

Select a tile to use shared preview/file commands; double-click a folder to enter
it. Arrow keys move spatially and Home/End select the first/last rendered item.
Right click uses the shared menu without opening a preview. Rename uses shared
inline-edit validation and operations. **Other** opens the normal file list;
the group is not a path that can be deleted. Zero-byte items have no map area
and remain available in List or Columns.

Normal browser folders remember their explicit view independently. Unconfigured
children open in Columns. Returning to a configured parent restores its view.
Choosing Columns removes only the mode override, preserving sort/icon-size fields.
Mode overrides survive the ephemeral settings LRU and cache clear. They are
independent of the optional remembered-sort/icon-size switch. Legacy global view
preferences do not seed unconfigured normal folders; the chooser retains its
separate policy and does not offer Disk Usage.

## Coverage and resource limits

Partial snapshots show measured progress; their percentages use the known total.
Complete means the bounded scan finished without a known omission. Updating marks
last-known data being reconciled. Unavailable does not mean zero bytes. Layout
updates interpolate briefly; navigation and reduced motion do not wait for them.

Enumeration, identity checks, aggregation and SQLite operations run in a background
service. Completed subtree measurements are shared, with a memory cache and a
separate SQLite WAL database in the application's cache directory. Identity checks
prevent reuse for a different filesystem object at the same path. Windows retains
the full 128-bit file ID and volume identity. Unsupported identities still allow
scanning but cannot establish a verified cache hit.

Native change events are coalesced. Refresh invalidates the current usage scope.
The existing Settings/tray cache-clear command also clears usage snapshots,
preserving source files and folder-mode preferences. Persisted data is last-known
until reconciled after restart. An unreadable/corrupt database falls back to memory
scanning; clearing it can still report an error instead of claiming success.

Scans currently stop at 200,000 nodes, 4,000 directories, 120 seconds or the retained
name budget and report Partial coverage. The canvas receives at most 300 direct
children and an aggregate tail. Watch registrations are bounded. Symlinks,
junctions/reparse points, cross-device boundaries and cloud placeholders are not
traversed. Recognized network locations do not start an automatic scan.

Allocated-space estimates, hard-link reclaim estimates, USN journal reconciliation
and remote scans are not offered. This is not a whole-volume always-current index.
See the [design contract](disk-usage-design.md) for the longer-term architecture
and validation requirements.
