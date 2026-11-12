# Text comparison

Select exactly two regular files in the same browser selection, right-click,
and choose **Compare text files**. This command displays a read-only unified
comparison in a modal. `-` marks removed lines and `+` marks added lines. The
header gives the original and new line ranges; up to three unchanged lines
surround the changed region.

Select part of the result and choose **Copy** to copy that selection. With no
selection, **Copy** copies the entire result. The source files are never changed.
Closing the modal or navigating away cancels the request and prevents a late
result from appearing.

Both inputs must be valid UTF-8 text. CRLF and LF line endings compare equally;
a missing final newline remains a difference. Binary, invalid UTF-8, unavailable
and unsupported inputs produce an explanation rather than an external tool launch.
On Windows, comparison accepts absolute paths on fixed local drives and refuses
leaf reparse points and offline/cloud-recall files. Files open for writing may
need to be closed before comparing.

Each input is limited to 1 MiB and 10,000 lines. The comparison also limits its
working matrix, processing time and result size (256 KiB). Only one comparison
runs at a time. These limits keep large or very different inputs from consuming
unbounded CPU or memory. Reading and comparison run outside the GTK thread.

This is the first implementation slice of the design imported from
[PR #38](https://github.com/IKurokami/strata/pull/38). The remaining
[Flea-inspired roadmap](flea-windows/00-roadmap.md) is separate work; importing
those documents does not enable the other proposed features.
