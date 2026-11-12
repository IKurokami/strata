# Export images from the file browser

Right-click your selection and hover over **Images** to open the export submenu.

Right-click one JPEG, BMP, single-frame GIF, or static WebP and choose
**Convert image to PNG** to save a PNG copy alongside it. The conversion
applies image orientation and preserves supported PNG color profiles.

Right-click one 8-bit PNG, JPEG, BMP, or single-frame GIF and choose
**Convert image to WebP** to save a lossless WebP copy with transparency.
The WebP export accepts RGB color profiles.

Right-click a PNG, BMP, single-frame GIF, or static WebP and choose
**Convert image to JPEG** to save a `.jpg` copy. Strata uses JPEG quality
90, composites transparency on white, and accepts RGB color profiles.

Select 2–20 images eligible for the same format and choose **Convert images to
PNG**, **Convert images to WebP**, or **Convert images to JPEG**. Strata checks the output names before
starting and processes the images in sequence. If an image fails or the dialog
is closed during conversion, copies that were already saved remain in place.

Select 2–20 PNG, JPEG, or WebP images from the same folder and choose
**Create PNG contact sheet**. Choose a grid, one row, or one column.
Strata places thumbnails in selection order on a transparent background.
In grid mode, choose the number of columns. Strata displays the resulting
row count, so four images with two columns form a 2 × 2 grid. All layouts
use thumbnails up to 256 pixels per side. Enter a filename ending in `.png`, or keep the default
`contact-sheet.png`. The result is saved alongside the originals.

Both actions show the destination filename before saving. The worker accepts
regular local files up to 32 MiB and 16 megapixels, and rejects animated inputs.
On Windows, it requires a fixed local drive and rejects reparse-point and
cloud-recall files. Closing the dialog cancels work before publication where
possible. An existing destination is preserved, including one created while
the export is running.
