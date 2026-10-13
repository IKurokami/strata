#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
#
# Bundle a native UCRT64 Windows developer build without silently depending
# on DLLs from the build runner's global MSYS2 PATH.
# Run in a genuine MSYS2 UCRT64 shell, after cargo build --bin strata.
set -euo pipefail

if [[ $# -ne 2 ]]; then
  printf 'Usage: %s /path/to/strata.exe /path/to/output-directory\n' "$0" >&2
  exit 2
fi

executable=$1
output=$2
prefix=${MINGW_PREFIX:-/ucrt64}

if [[ ! -s "$executable" ]]; then
  printf 'Missing linked Windows executable: %s\n' "$executable" >&2
  exit 1
fi
if [[ "$prefix" != /ucrt64 ]]; then
  printf 'Expected UCRT64 toolchain, got: %s\n' "$prefix" >&2
  exit 1
fi
command -v objdump >/dev/null || { echo 'Missing PE objdump from UCRT64 binutils' >&2; exit 1; }

mkdir -p "$output/bin" "$output/share" "$output/lib"
cp "$executable" "$output/bin/strata.exe"

# Traversal is entirely build-time. Read PE import names, never execute DLLs.
# Resolve imports only from the trusted /ucrt64/bin installation, not PATH,
# the current directory, user-defined plugin directories or remote mounts.
declare -A queued=()
declare -a queue=()
enqueue() {
  local file=$1
  if [[ -z "${queued[$file]+set}" ]]; then
    queued[$file]=1
    queue+=("$file")
  fi
}

enqueue "$output/bin/strata.exe"

# Optional GIO/pixbuf modules are loadable at runtime rather than imports of
# the main executable. Their transitive DLL dependencies must also be bundled.
for module in gio/modules gdk-pixbuf-2.0/2.10.0/loaders; do
  if [[ -d "$prefix/lib/$module" ]]; then
    mkdir -p "$output/lib/$module"
    cp -R "$prefix/lib/$module/." "$output/lib/$module/"
    while IFS= read -r -d '' plugin; do
      enqueue "$plugin"
    done < <(find "$output/lib/$module" -type f -iname '*.dll' -print0)
  fi
done

# Package enough GLib/icon/MIME metadata for file browsing. Do not bundle
# MSYS2's /usr/bin, unrelated developer tools, the Rust toolchain or shells.
for item in glib-2.0/schemas mime icons/Adwaita; do
  if [[ -d "$prefix/share/$item" ]]; then
    mkdir -p "$output/share/$item"
    cp -R "$prefix/share/$item/." "$output/share/$item/"
  fi
done

processed=0
while (( processed < ${#queue[@]} )); do
  file=${queue[$processed]}
  processed=$((processed + 1))
  while IFS= read -r dependency; do
    [[ -n "$dependency" ]] || continue
    name=${dependency,,}
    if [[ "$name" == msys-2.0.dll ]]; then
      printf 'Refusing to ship an MSYS2-runtime-dependent module: %s\n' "$file" >&2
      exit 1
    fi
    # Windows system imports legitimately do not exist in /ucrt64/bin.
    # An isolated-path smoke test is required to catch unknown missing DLLs.
    candidate=$(find "$prefix/bin" -maxdepth 1 -type f -iname "$dependency" -print -quit)
    [[ -n "$candidate" ]] || continue
    destination="$output/bin/$(basename "$candidate")"
    if [[ ! -f "$destination" ]]; then
      cp "$candidate" "$destination"
      enqueue "$destination"
    fi
  done < <(objdump -p "$file" | awk 'tolower($1) == "dll" && tolower($2) == "name:" {print $3}')
done

# The user can double-click this launcher. No UAC, installer, shell tools,
# registry writes, service startup or file-system scan is performed by Strata.
cat > "$output/Run-Strata.cmd" <<'CMD'
@echo off
setlocal
set "STRATA_ROOT=%~dp0"
set "PATH=%STRATA_ROOT%bin;%PATH%"
set "GSETTINGS_SCHEMA_DIR=%STRATA_ROOT%share\glib-2.0\schemas"
set "XDG_DATA_DIRS=%STRATA_ROOT%share"
set "GIO_EXTRA_MODULES=%STRATA_ROOT%lib\gio\modules"
"%STRATA_ROOT%bin\strata.exe" %*
exit /b %ERRORLEVEL%
CMD

cat > "$output/README-WINDOWS-DEVELOPMENT.txt" <<'TXT'
Strata for Windows — UCRT64 developer-only experimental package.

Run Run-Strata.cmd to use bundled native DLLs.
This package is NOT a production-ready portable release:
- GUI first paint and Windows Explorer integration need interactive testing.
- Custom actions, unsafe rich-media decoders, Recycle Bin and Windows
  self-update intentionally fail closed until native implementations exist.
- The package is not code-signed and does not install or request elevation.
- Check that your antivirus and organization policies permit local executables.
TXT

printf 'Bundled %s scanned PE images in %s\n' "$processed" "$output"
