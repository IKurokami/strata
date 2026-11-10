# Export images from the file browser

Right-click one JPEG, BMP, single-frame GIF, or static WebP and choose
**Convert image to PNG** to save a PNG copy alongside it. The conversion
applies image orientation and preserves supported PNG color profiles.

Select 2–20 PNG, JPEG, or WebP images from the same folder and choose
**Create PNG contact sheet**. Strata places thumbnails in selection order
in a transparent grid and saves `contact-sheet.png` alongside the originals.
The grid uses up to four columns and thumbnails up to 256 pixels per side.

Both actions show the destination filename before saving. The worker accepts
regular local files up to 32 MiB and 16 megapixels, and rejects animated inputs.
On Windows, it requires a fixed local drive and rejects reparse-point and
cloud-recall files. Closing the dialog cancels work before publication where
possible. An existing destination is preserved, including one created while
the export is running.
