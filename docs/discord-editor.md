# Discord text editing

Local text and source-code previews have an **Edit for Discord** pencil action.
Ordinary preview stays read-only and does not rewrite text or interpret ANSI.
The editor is a modal in the existing Strata window; browsing to another item
does not retarget a document already being edited.

Choose **ANSI**, **Markdown**, **Code**, or **Plain text**. A complete ANSI block
opens as colored, editable text. Selecting ANSI for ordinary text creates a
fenced `ansi` draft; it writes the fences and actual ESC characters only when
Save is pressed. Existing outer fences are not nested. Code mode has an editable
language identifier and creates a fenced code block on copy/save.

The ANSI document uses a dark terminal canvas so explicit white text stays visible
in light themes; the surrounding controls follow the application's theme.
ANSI tools apply to selected text: bold, underline, foreground/background colors,
custom RGB colors, gradient, and reset. Mixed colors outside the selection are
retained. Imports accept basic/bright/indexed/RGB colors, resets, bold, italic,
underline, and strikethrough. Unsupported control sequences are rejected rather
than executed or silently discarded. ANSI support and color appearance depend
on the Discord client; Strata's preview is an approximation, not Discord itself.

Markdown tools cover bold, italic, underline, strikethrough, spoilers, inline code,
links, three heading levels, subtext, single/multiline quotes, bullet/numbered
lists, and code blocks. Preview renders these basic formats; code is literal and
spoilers are masked. It does not contact Discord, open links, or resolve mentions.
The syntax follows [Discord's formatting guide](https://support.discord.com/hc/en-us/articles/210298617-Markdown-Text-101-Chat-Formatting-Bold-Italic-Underline).

Copy exports plain Unicode clipboard text, including the required fences/escape
characters. The counter includes those characters and shows Discord's ordinary
2,000-character and Nitro 4,000-character limits. Long drafts are not automatically
split. Payloads containing a code-block delimiter cannot be exported in ANSI/Code
mode because they would terminate the Discord block.

Undo/redo includes formatting and mode changes as well as text. Closing a dirty
draft keeps it open until changes are saved or explicitly discarded. Saving
preserves the original UTF-8 BOM and newline convention. Reads/writes run off
the GTK thread; a byte comparison plus GIO's expected etag protects against
overwriting external changes. Files with mixed newlines are normalized to CRLF
if the original contains CRLF. The editor accepts regular UTF-8 files up to
64 KiB and up to 4,096 styled runs. Larger files remain available in the normal
bounded preview. Markdown preview has a parsing budget and bounded nesting;
undo history is bounded to 32 snapshots and approximately 4 MiB.
