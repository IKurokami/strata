#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Stage only the pinned embedding worker and its libraries, never the DigUp UI."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import urllib.request

ROOT = Path(__file__).resolve().parent.parent


def sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def stage(destination, platform, cache):
    manifest = json.loads((ROOT / "packaging/smart-search/runtime.json").read_text())
    asset = manifest[platform]
    cache.mkdir(parents=True, exist_ok=True)
    archive = cache / Path(asset["url"]).name
    if not archive.exists() or archive.stat().st_size != asset["bytes"] or sha256(archive) != asset["sha256"]:
        temporary = cache / (archive.name + ".download")
        with urllib.request.urlopen(asset["url"], timeout=60) as source, temporary.open("wb") as output:
            size = 0
            while block := source.read(1024 * 1024):
                size += len(block)
                if size > asset["bytes"]:
                    raise ValueError("Embedding runtime exceeds its pinned size")
                output.write(block)
        if temporary.stat().st_size != asset["bytes"] or sha256(temporary) != asset["sha256"]:
            raise ValueError("Embedding runtime checksum mismatch")
        temporary.replace(archive)
    extractor = shutil.which("7z") or shutil.which("7zz")
    if not extractor and Path("C:/Program Files/7-Zip/7z.exe").is_file():
        extractor = "C:/Program Files/7-Zip/7z.exe"
    if not extractor:
        raise RuntimeError("7-Zip is required to stage the verified runtime")
    with tempfile.TemporaryDirectory(prefix="embedding-runtime-", dir=cache) as directory:
        unpacked = Path(directory)
        subprocess.run([extractor, "x", str(archive), "-o" + str(unpacked), "-y"], check=True, stdout=subprocess.DEVNULL)
        if platform == "linux":
            data = next(unpacked.glob("data.tar*"), None)
            if data is None:
                raise RuntimeError("The pinned Debian package has no data archive")
            subprocess.run([extractor, "x", str(data), "-o" + str(unpacked / "data"), "-y"], check=True, stdout=subprocess.DEVNULL)
            if data.suffix != ".tar":
                inner = next((unpacked / "data").glob("*.tar"), None)
                if inner is None:
                    raise RuntimeError("The pinned Debian data archive is invalid")
                subprocess.run([extractor, "x", str(inner), "-o" + str(unpacked / "files"), "-y"], check=True, stdout=subprocess.DEVNULL)
        executable = "digup.exe" if platform == "windows" else "digup"
        workers = [path for path in unpacked.rglob(executable) if path.parent.name == "helper"]
        if len(workers) != 1:
            raise RuntimeError("The pinned package did not contain exactly one helper")
        worker = workers[0]
        selected = [path for path in worker.parent.rglob("*") if path.is_file() and
                    (path == worker or path.suffix.lower() == ".dll" or ".so" in path.name)]
        selected += [path for path in unpacked.rglob("THIRD-PARTY-NOTICES.txt") if path.is_file()]
        if not selected:
            raise RuntimeError("Embedding runtime is empty")
        destination.mkdir(parents=True, exist_ok=True)
        entries = []
        for source in selected:
            relative = source.relative_to(worker.parent) if source.is_relative_to(worker.parent) else Path(source.name)
            output = destination / relative
            output.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, output)
            if output.name == "digup":
                output.chmod(0o755)
            entries.append({"path": relative.as_posix(), "bytes": output.stat().st_size, "sha256": sha256(output)})
        (destination / "runtime-manifest.json").write_text(json.dumps({
            "format": 1, "source_revision": manifest["revision"], "source_sha256": asset["sha256"], "files": entries,
        }, indent=2) + "\n")
    print(f"Staged {len(entries)} embedding runtime files; no model weights or desktop shell included.")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--destination", type=Path, required=True)
    parser.add_argument("--platform", choices=["windows", "linux"], required=True)
    parser.add_argument("--cache", type=Path, default=ROOT / "target/smart-search-runtime-cache")
    arguments = parser.parse_args()
    stage(arguments.destination.resolve(), arguments.platform, arguments.cache.resolve())
