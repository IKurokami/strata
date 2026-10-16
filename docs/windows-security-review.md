# Windows security review for PR #26

This review records the enforced boundaries in the Windows implementation and
the evidence produced by the Windows workflow. It is intentionally separate
from Linux sandbox claims.

## Enforced controls

- Preview helpers run in a Win32 Job Object with a process limit, memory limit,
  UI restrictions, timeout and kill-on-close behavior.
- FFmpeg and FFprobe media workers use the streaming Job Object API; replacing
  or cancelling a preview terminates and reaps every decoder.
- Provider roots, manifests, icons and executables must be regular,
  non-reparse files owned by the current Windows token. Provider requests use a
  bounded newline protocol and a per-request Job Object.
- Native copy and verified cross-volume move reject reparse sources, publish
  through no-clobber staging and delete a source only after destination
  verification.
- Recycle Bin and permanent deletion are separate COM STA operations.
- Thumbnail cache entries are bounded, keyed by volume/file identity and
  validated before returning bytes to GTK.

## Acceptance evidence

The linked Windows workflow must provide the GUI first-paint JSON, portable
release manifest, native regression output and the final same-SHA CI URLs.
Security-sensitive fault injection remains a required release gate for
reparse-point races, source replacement, destination races, cancellation,
offline mapped drives and partial failures.
