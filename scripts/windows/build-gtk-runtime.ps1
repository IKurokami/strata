# SPDX-License-Identifier: MIT
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$BuildRoot,
    [Parameter(Mandatory = $true)][string]$Destination,
    [string]$UcrtRoot = 'C:/msys64/ucrt64',
    [string]$Meson = 'meson',
    [int]$Jobs = 8
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$BuildRoot = [IO.Path]::GetFullPath($BuildRoot)
$Destination = [IO.Path]::GetFullPath($Destination)
if (Test-Path -LiteralPath $BuildRoot) { throw 'Use a fresh GTK build directory.' }
if (Test-Path -LiteralPath $Destination) { throw 'Use a fresh runtime destination.' }
$env:PATH = (Join-Path $UcrtRoot 'bin') + ';' + $env:PATH
$env:CC = 'gcc'
$env:CXX = 'g++'
$env:PKG_CONFIG = Join-Path $UcrtRoot 'bin/pkgconf.exe'
$gstVersion = & $env:PKG_CONFIG --modversion gstreamer-play-1.0
if ($LASTEXITCODE -ne 0 -or $gstVersion -ne '1.28.8') { throw 'This private GstPlay patch requires the matching GStreamer 1.28.8 baseline.' }
$kit = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../packaging/windows/runtime'))
New-Item -ItemType Directory -Path $BuildRoot | Out-Null
$archive = Join-Path $BuildRoot 'gtk-4.24.1.tar.xz'
$sourceHash = 'e98abe720e16129c8c0f50761dee0a2e9ae2478055e31018f6cf977fcf6513e9'
Invoke-WebRequest 'https://download.gnome.org/sources/gtk/4.24/gtk-4.24.1.tar.xz' -OutFile $archive
if ((Get-FileHash $archive).Hash -ine $sourceHash) { throw 'GTK source checksum mismatch.' }
$fontPatch = Join-Path $BuildRoot '001-fix-font-rendering.patch'
Invoke-WebRequest 'https://raw.githubusercontent.com/msys2/MINGW-packages/master/mingw-w64-gtk4/001-fix-font-rendering.patch' -OutFile $fontPatch
if ((Get-FileHash $fontPatch).Hash -ine 'ebac78616a7668edbfdd77b9959ad63239aa59ed12b995a9663bf2df7158be26') { throw 'MSYS2 font patch checksum mismatch.' }
& tar -xf $archive -C $BuildRoot
if ($LASTEXITCODE -ne 0) { throw 'GTK source extraction failed.' }
$source = Join-Path $BuildRoot 'gtk-4.24.1'
# Prevent git apply from discovering Strata's parent repository when BuildRoot
# is under target/, where it could silently skip the upstream-relative paths.
& git init --quiet $source
if ($LASTEXITCODE -ne 0) { throw 'Cannot isolate GTK patch application.' }
$patch = Join-Path $kit 'gtk-4.24.1-ime-lifetime.patch'
foreach ($inputPatch in @($fontPatch, $patch)) {
    & git -C $source apply --check $inputPatch
    if ($LASTEXITCODE -ne 0) { throw "GTK patch does not apply exactly: $inputPatch" }
    & git -C $source apply $inputPatch
    if ($LASTEXITCODE -ne 0) { throw 'GTK patch application failed.' }
}
$build = Join-Path $BuildRoot 'build'
& $Meson setup $build $source "--prefix=$UcrtRoot" --wrap-mode=nofallback --buildtype=release `
    -Dbuild-demos=false -Dbuild-tests=false -Dbuild-testsuite=false -Dbuild-examples=false `
    -Dintrospection=disabled -Ddocumentation=false -Dman-pages=false `
    -Dmedia-gstreamer=enabled -Dvulkan=enabled -Dx11-backend=false -Dwayland-backend=false `
    -Dmacos-backend=false -Dwin32-backend=true
if ($LASTEXITCODE -ne 0) { throw 'GTK configuration failed.' }
& ninja -C $build -j $Jobs gtk/libgtk-4-1.dll
if ($LASTEXITCODE -ne 0) { throw 'GTK build failed.' }
$gstArchive = Join-Path $BuildRoot 'gst-plugins-bad-1.28.8.tar.xz'
$gstSourceHash = '670d888bde8242579b0d8e91f60cf337aa7be20ca7c315ebfbdd6e5a9218d104'
Invoke-WebRequest 'https://gstreamer.freedesktop.org/src/gst-plugins-bad/gst-plugins-bad-1.28.8.tar.xz' -OutFile $gstArchive
if ((Get-FileHash $gstArchive).Hash -ine $gstSourceHash) { throw 'GStreamer source checksum mismatch.' }
& tar -xf $gstArchive -C $BuildRoot
if ($LASTEXITCODE -ne 0) { throw 'GStreamer source extraction failed.' }
$gstSource = Join-Path $BuildRoot 'gst-plugins-bad-1.28.8'
& git init --quiet $gstSource
if ($LASTEXITCODE -ne 0) { throw 'Cannot isolate GStreamer patch application.' }
$gstPatch = Join-Path $kit 'gstreamer-1.28.8-defer-worker-finalize.patch'
& git -C $gstSource apply --check $gstPatch
if ($LASTEXITCODE -ne 0) { throw 'GstPlay patch does not apply exactly.' }
& git -C $gstSource apply $gstPatch
if ($LASTEXITCODE -ne 0) { throw 'GstPlay patch application failed.' }
$gstBuild = Join-Path $BuildRoot 'gst-build'
& $Meson setup $gstBuild $gstSource "--prefix=$UcrtRoot" --wrap-mode=nofallback --buildtype=release `
    -Dauto_features=disabled -Dintrospection=disabled -Dtests=disabled -Dexamples=disabled -Ddoc=disabled
if ($LASTEXITCODE -ne 0) { throw 'GstPlay configuration failed.' }
& ninja -C $gstBuild -j $Jobs gst-libs/gst/play/libgstplay-1.0-0.dll
if ($LASTEXITCODE -ne 0) { throw 'GstPlay build failed.' }
# Keep the exact corresponding source and modifications with the private runtime.
New-Item -ItemType Directory -Path $Destination | Out-Null
Copy-Item (Join-Path $build 'gtk/libgtk-4-1.dll') $Destination
Copy-Item (Join-Path $gstBuild 'gst-libs/gst/play/libgstplay-1.0-0.dll') $Destination
Copy-Item (Join-Path $gstSource 'COPYING') (Join-Path $Destination 'COPYING-gstreamer')
foreach ($inputFile in @($archive, $fontPatch, $patch, (Join-Path $source 'COPYING'))) {
    Copy-Item -LiteralPath $inputFile -Destination $Destination
}
foreach ($inputFile in @($gstArchive, $gstPatch)) { Copy-Item -LiteralPath $inputFile -Destination $Destination }
[pscustomobject]@{
    format = 'strata-private-gtk-v1'
    version = '4.24.1'
    architecture = 'x86_64'
    source_sha256 = $sourceHash
    patch_sha256 = (Get-FileHash $patch).Hash.ToLowerInvariant()
    dll_sha256 = (Get-FileHash (Join-Path $Destination 'libgtk-4-1.dll')).Hash.ToLowerInvariant()
    gstreamer_version = '1.28.8'
    gstreamer_source_sha256 = $gstSourceHash
    gstreamer_patch_sha256 = (Get-FileHash $gstPatch).Hash.ToLowerInvariant()
    gstplay_dll_sha256 = (Get-FileHash (Join-Path $Destination 'libgstplay-1.0-0.dll')).Hash.ToLowerInvariant()
    compiler = (& gcc --version | Select-Object -First 1)
    meson = (& $Meson --version)
} | ConvertTo-Json | Set-Content (Join-Path $Destination 'runtime.json') -Encoding UTF8
Write-Output "Private GTK runtime: $Destination"
