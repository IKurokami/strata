#!/usr/bin/env python3
"""Convert the existing Strata application SVG to a multi-resolution Windows ICO."""

import argparse
from pathlib import Path
import struct
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--renderer", default="rsvg-convert")
    options = parser.parse_args()
    root = Path(__file__).resolve().parents[2]
    source = root / "data/icons/scalable/apps/io.github.lgse.Strata.svg"
    sizes = (16, 20, 24, 32, 40, 48, 64, 128, 256)
    images = [
        subprocess.check_output(
            [options.renderer, "--width", str(size), "--height", str(size), str(source)]
        )
        for size in sizes
    ]
    offset = 6 + 16 * len(images)
    entries = []
    for size, image in zip(sizes, images):
        entries.append(struct.pack("<BBBBHHII", size % 256, size % 256, 0, 0,
                                   1, 32, len(image), offset))
        offset += len(image)
    (root / "data/brand/strata.ico").write_bytes(
        struct.pack("<HHH", 0, 1, len(images)) + b"".join(entries) + b"".join(images)
    )


if __name__ == "__main__":
    main()
