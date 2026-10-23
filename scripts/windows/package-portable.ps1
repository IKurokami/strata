# SPDX-License-Identifier: MIT
# Relocatable Windows/UCRT64 runtime bundle. Must run on a Windows MSYS2
# UCRT64 build runner after strata.exe is linked. Use -Release after the
# blocking GUI and native regression gates have passed.
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Executable,
    [Parameter(Mandatory = $true)][string]$UcrtRoot,
    [Parameter(Mandatory = $true)][string]$Destination,
    [string]$GtkRuntime = (Join-Path $PSScriptRoot '../../target/windows-gtk-runtime'),
    [switch]$Release
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$exe = (Resolve-Path -LiteralPath $Executable).Path
$prefix = (Resolve-Path -LiteralPath $UcrtRoot).Path
$Destination = [System.IO.Path]::GetFullPath($Destination)
$runtimeBin = Join-Path $prefix "bin"
$objdump = Join-Path $runtimeBin "objdump.exe"
if (-not (Test-Path -LiteralPath $objdump)) {
    throw "UCRT64 objdump.exe is required to verify every imported runtime DLL"
}
$autoAudioPlugin = Join-Path $prefix "lib/gstreamer-1.0/libgstautodetect.dll"
if (-not (Test-Path -LiteralPath $autoAudioPlugin)) {
    throw "Required GStreamer audio plugin missing: $autoAudioPlugin. Install mingw-w64-ucrt-x86_64-gst-plugins-good."
}
if (-not (Test-Path -LiteralPath (Join-Path $prefix 'lib/gstreamer-1.0/libgstlibav.dll'))) {
    throw 'Required GStreamer H.264/audio decoder missing. Install mingw-w64-ucrt-x86_64-gst-libav.'
}

if (Test-Path -LiteralPath $Destination) {
    throw "Destination already exists. Choose a new staging directory: $Destination"
}
$stageBin = Join-Path $Destination "bin"
New-Item -ItemType Directory -Path $stageBin -Force | Out-Null
Copy-Item -LiteralPath $exe -Destination (Join-Path $stageBin "strata.exe")
$shellHelper = Join-Path (Split-Path -Parent $exe) "strata-shell.exe"
if (-not (Test-Path -LiteralPath $shellHelper -PathType Leaf)) {
    throw "Standalone shell helper missing. Build strata-shell together with strata using cargo build --bins."
}
Copy-Item -LiteralPath $shellHelper -Destination (Join-Path $stageBin "strata-shell.exe")
Copy-Item -LiteralPath $shellHelper -Destination (Join-Path $Destination "Start-Strata.exe")
if (Test-Path -LiteralPath $GtkRuntime -PathType Container) {
    $runtime = Get-Content -LiteralPath (Join-Path $GtkRuntime 'runtime.json') -Raw | ConvertFrom-Json
    $patch = Join-Path $PSScriptRoot '../../packaging/windows/runtime/gtk-4.24.1-ime-lifetime.patch'
    $privateGtk = Join-Path $GtkRuntime 'libgtk-4-1.dll'
    $privateGst = Join-Path $GtkRuntime 'libgstplay-1.0-0.dll'
    $gstPatch = Join-Path $PSScriptRoot '../../packaging/windows/runtime/gstreamer-1.28.8-defer-worker-finalize.patch'
    if ($runtime.format -ne 'strata-private-gtk-v1' -or $runtime.version -ne '4.24.1' -or
        $runtime.architecture -ne 'x86_64' -or
        $runtime.source_sha256 -ne 'e98abe720e16129c8c0f50761dee0a2e9ae2478055e31018f6cf977fcf6513e9' -or
        $runtime.patch_sha256 -ine (Get-FileHash $patch).Hash -or
        $runtime.dll_sha256 -ine (Get-FileHash $privateGtk).Hash -or
        $runtime.gstreamer_version -ne '1.28.8' -or
        $runtime.gstreamer_source_sha256 -ne '670d888bde8242579b0d8e91f60cf337aa7be20ca7c315ebfbdd6e5a9218d104' -or
        $runtime.gstreamer_patch_sha256 -ine (Get-FileHash $gstPatch).Hash -or
        $runtime.gstplay_dll_sha256 -ine (Get-FileHash $privateGst).Hash) {
        throw 'Private GTK provenance does not match the pinned source, patch, and DLL.'
    }
    Copy-Item -LiteralPath $privateGtk -Destination (Join-Path $stageBin 'libgtk-4-1.dll')
    Copy-Item -LiteralPath $privateGst -Destination (Join-Path $stageBin 'libgstplay-1.0-0.dll')
    $runtimeNotice = Join-Path $Destination 'share/strata/runtime'
    New-Item -ItemType Directory -Path $runtimeNotice -Force | Out-Null
    foreach ($name in @('runtime.json', 'gtk-4.24.1.tar.xz', '001-fix-font-rendering.patch', 'gtk-4.24.1-ime-lifetime.patch', 'COPYING', 'gst-plugins-bad-1.28.8.tar.xz', 'gstreamer-1.28.8-defer-worker-finalize.patch', 'COPYING-gstreamer')) {
        Copy-Item -LiteralPath (Join-Path $GtkRuntime $name) -Destination $runtimeNotice
    }
    if ((Get-FileHash (Join-Path $runtimeNotice 'gtk-4.24.1.tar.xz')).Hash -ine $runtime.source_sha256 -or
        (Get-FileHash (Join-Path $runtimeNotice 'gtk-4.24.1-ime-lifetime.patch')).Hash -ine $runtime.patch_sha256 -or
        (Get-FileHash (Join-Path $runtimeNotice '001-fix-font-rendering.patch')).Hash -ine 'ebac78616a7668edbfdd77b9959ad63239aa59ed12b995a9663bf2df7158be26' -or
        (Get-FileHash (Join-Path $runtimeNotice 'gst-plugins-bad-1.28.8.tar.xz')).Hash -ine $runtime.gstreamer_source_sha256 -or
        (Get-FileHash (Join-Path $runtimeNotice 'gstreamer-1.28.8-defer-worker-finalize.patch')).Hash -ine $runtime.gstreamer_patch_sha256) {
        throw 'Private GTK corresponding source or patches do not match the provenance.'
    }
} elseif ($Release) {
    throw 'Release packaging requires the private Windows IME runtime. Run scripts/windows/build-gtk-runtime.ps1 first.'
}
# Ship the media decoders alongside Strata; no machine-level codec installation.
foreach ($name in @("ffmpeg.exe", "ffprobe.exe")) {
    $source = Join-Path $runtimeBin $name
    if (-not (Test-Path -LiteralPath $source)) {
        throw "Required bundled media decoder missing: $source"
    }
    Copy-Item -LiteralPath $source -Destination (Join-Path $stageBin $name)
}

# GTK/GIO/GdkPixbuf/GStreamer resources are loaded dynamically. Keep the
# MSYS2-relative layout so the staged launcher can resolve them locally.
$runtimeDirectories = @(
    "share/glib-2.0/schemas",
    "share/gtk-4.0",
    "share/icons/Adwaita",
    "share/icons/hicolor",
    "share/mime",
    "share/gtksourceview-5",
    "share/fontconfig",
    "etc/fonts",
    "lib/gio/modules",
    "lib/gdk-pixbuf-2.0",
    "lib/gstreamer-1.0",
    "share/licenses"
)
foreach ($relative in $runtimeDirectories) {
    $from = Join-Path $prefix ($relative -replace '/', [System.IO.Path]::DirectorySeparatorChar)
    if (-not (Test-Path -LiteralPath $from)) {
        if ($relative -eq "share/glib-2.0/schemas") {
            throw "Required GTK/GSettings runtime schemas missing: $from"
        }
        continue
    }
    $to = Join-Path $Destination ($relative -replace '/', [System.IO.Path]::DirectorySeparatorChar)
    New-Item -ItemType Directory -Path (Split-Path -Parent $to) -Force | Out-Null
    Copy-Item -LiteralPath $from -Destination $to -Recurse -Force
}

$scanner = Join-Path $prefix "libexec/gstreamer-1.0/gst-plugin-scanner.exe"
if (-not (Test-Path -LiteralPath $scanner)) {
    throw "Required native GStreamer plugin scanner missing: $scanner"
}
$scannerDest = Join-Path $Destination "libexec/gstreamer-1.0/gst-plugin-scanner.exe"
New-Item -ItemType Directory -Path (Split-Path -Parent $scannerDest) -Force | Out-Null
Copy-Item -LiteralPath $scanner -Destination $scannerDest

# Walk imports for the executable and dynamically loaded native modules. Do
# not bundle an MSYS2 POSIX runtime or silently accept an unknown import.
$visited = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::OrdinalIgnoreCase)
$queued = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::OrdinalIgnoreCase)
$queue = [System.Collections.Generic.Queue[string]]::new()
function Queue-Binary([string]$Path) {
    if ($queued.Add($Path)) { $queue.Enqueue($Path) }
}
Queue-Binary (Join-Path $stageBin "strata.exe")
Queue-Binary (Join-Path $stageBin "strata-shell.exe")
Queue-Binary (Join-Path $stageBin "ffmpeg.exe")
Queue-Binary (Join-Path $stageBin "ffprobe.exe")
Queue-Binary $scannerDest
foreach ($relative in @("lib/gio", "lib/gdk-pixbuf-2.0", "lib/gstreamer-1.0")) {
    $dir = Join-Path $Destination $relative
    if (Test-Path -LiteralPath $dir) {
        Get-ChildItem -LiteralPath $dir -Recurse -File -Filter "*.dll" |
            ForEach-Object { Queue-Binary $_.FullName }
    }
}

$importCount = 0
while ($queue.Count -gt 0) {
    $binary = $queue.Dequeue()
    if (-not $visited.Add($binary)) { continue }
    $imports = @(& $objdump -p $binary 2>&1)
    if ($LASTEXITCODE -ne 0) {
        throw "Failed reading PE import table of $binary"
    }
    foreach ($line in $imports) {
        if ($line -notmatch '^\s*DLL Name:\s*(\S+)\s*$') { continue }
        $dll = $Matches[1]
        if ($dll -notmatch '^[\w.+-]+\.(dll|drv|ocx|cpl)$') {
            throw "Suspicious DLL import name '$dll' in $binary"
        }
        $importCount++
        if ($dll -ieq "msys-2.0.dll") {
            throw "MSYS2 POSIX-runtime dependency in portable native package: $binary"
        }
        $localDll = Join-Path $runtimeBin $dll
        if (Test-Path -LiteralPath $localDll) {
            $packaged = Join-Path $stageBin $dll
            if (-not (Test-Path -LiteralPath $packaged)) {
                Copy-Item -LiteralPath $localDll -Destination $packaged
            }
            Queue-Binary $packaged
            continue
        }
        if ($dll -match '^(api-ms-win-|ext-ms-win-)') { continue }
        $windowsDll = Join-Path (Join-Path $env:SystemRoot "System32") $dll
        if (-not (Test-Path -LiteralPath $windowsDll)) {
            throw "Unresolved PE import '$dll' in '$binary'. Refusing incomplete distribution."
        }
    }
}
if ((Get-ChildItem -LiteralPath $stageBin -File -Filter "*.dll").Count -lt 5) {
    throw "Suspiciously few native dependencies were bundled"
}

# Generate a relocatable GdkPixbuf loader cache. The source cache may contain
# absolute MSYS2 paths and must never be shipped.
$loaderDir = Join-Path $Destination "lib/gdk-pixbuf-2.0/2.10.0/loaders"
$queryLoaders = Join-Path $runtimeBin "gdk-pixbuf-query-loaders.exe"
if (-not (Test-Path -LiteralPath $queryLoaders)) {
    throw "GdkPixbuf cache generation tool is missing: $queryLoaders"
}
if (-not (Test-Path -LiteralPath $loaderDir)) {
    throw "GdkPixbuf loader directory is missing: $loaderDir"
}
$loaderFiles = @(Get-ChildItem -LiteralPath $loaderDir -File -Filter "*.dll" |
    ForEach-Object { $_.FullName })
if ($loaderFiles.Count -eq 0) {
    throw "GdkPixbuf modules were not bundled"
}
$cacheLines = @(& $queryLoaders @loaderFiles 2>&1)
if ($LASTEXITCODE -ne 0) {
    throw "gdk-pixbuf-query-loaders failed to inspect staged modules"
}
$cacheEntries = 0
$relocatableCache = @(
    foreach ($item in $cacheLines) {
        $line = [string]$item
        if ($line -match '^"([^"]+\.dll)"') {
            $name = [System.IO.Path]::GetFileName($Matches[1])
            $cacheEntries++
            '"lib/gdk-pixbuf-2.0/2.10.0/loaders/' + $name + '"'
        } else {
            $line
        }
    }
)
if ($cacheEntries -eq 0) {
    throw "No GdkPixbuf module entries found in generated loader cache"
}
$cacheFile = Join-Path (Split-Path -Parent $loaderDir) "loaders.cache"
$relocatableCache | Set-Content -LiteralPath $cacheFile -Encoding UTF8
if (Select-String -LiteralPath $cacheFile -Pattern 'msys64|ucrt64|^[A-Za-z]:' -Quiet) {
    throw "GdkPixbuf loader cache contains non-relocatable build paths"
}

$launcher = @'
@echo off
setlocal
set "STRATA_ATTACH_PARENT_CONSOLE=1"
rem A standalone Windows bundle has no session bus. Preserve an explicitly supplied bus.
if not defined DBUS_SESSION_BUS_ADDRESS set "DBUS_SESSION_BUS_ADDRESS=disabled:"
if not defined GDK_DEBUG (
    if not defined GDK_DISABLE set "GDK_DISABLE=d3d11,d3d12,gl,vulkan"
    if not defined GSK_RENDERER set "GSK_RENDERER=cairo"
)
set "PATH=%~dp0bin;%SystemRoot%\System32;%SystemRoot%"
set "GSETTINGS_SCHEMA_DIR=%~dp0share\glib-2.0\schemas"
set "GIO_EXTRA_MODULES=%~dp0lib\gio\modules"
set "GST_PLUGIN_SYSTEM_PATH=%~dp0lib\gstreamer-1.0"
set "GST_PLUGIN_SCANNER=%~dp0libexec\gstreamer-1.0\gst-plugin-scanner.exe"
set "GDK_PIXBUF_MODULE_FILE=%~dp0lib\gdk-pixbuf-2.0\2.10.0\loaders.cache"
set "GDK_PIXBUF_MODULEDIR=%~dp0lib\gdk-pixbuf-2.0\2.10.0\loaders"
set "FONTCONFIG_PATH=%~dp0etc\fonts"
set "STRATA_FFMPEG=%~dp0bin\ffmpeg.exe"
set "STRATA_FFPROBE=%~dp0bin\ffprobe.exe"
"%~dp0bin\strata.exe" %*
exit /b %ERRORLEVEL%
'@
$launcher = $launcher.Replace("`r`n", "`n").Replace("`n", "`r`n")
Set-Content -LiteralPath (Join-Path $Destination "Start-Strata.cmd") -Value $launcher -Encoding ASCII

$warning = if ($Release) {
@'
STRATA WINDOWS PORTABLE RELEASE

Start with Start-Strata.exe. The bundle contains the recursively linked UCRT64
runtime, GTK resources, GStreamer plugins and bundled FFmpeg tools; it does not
require MSYS2 on PATH. Keep this directory together when moving the application.

Shell integration is off by default. Enabling Replace File Explorer in Settings
registers per-user folder/drive handlers and a small sign-in helper for Win+E.
Disabling it restores the previous handlers. No administrator rights are needed. The
runtime DLLs and plugins are open-source third-party components with their own
licenses (see share/licenses).
'@
} else {
@'
STRATA WINDOWS DEVELOPER BUILD — NOT A RELEASE

Start with Start-Strata.exe, not from a globally installed MSYS2 terminal.
This bundle contains the recursively linked UCRT64 runtime and GTK resources.
Use the release switch only after the isolated GUI smoke and native regression
gates have passed on the same build.

The runtime DLLs and plugins are open-source third-party components with their
own licenses (see share/licenses); they are not authored by OpenAI or Strata.
'@
}
Set-Content -LiteralPath (Join-Path $Destination "READ-ME-FIRST.txt") -Value $warning -Encoding UTF8

# Run both the PE and launcher with only staged DLLs plus Windows system DLLs.
$start = [System.Diagnostics.ProcessStartInfo]::new()
$start.FileName = Join-Path $stageBin "strata.exe"
$start.Arguments = "--version"
$start.WorkingDirectory = $Destination
$start.UseShellExecute = $false
$start.RedirectStandardOutput = $true
$start.RedirectStandardError = $true
$start.Environment["PATH"] = "$stageBin;$($env:SystemRoot)\System32;$($env:SystemRoot)"
$start.Environment["STRATA_FFMPEG"] = Join-Path $stageBin "ffmpeg.exe"
$start.Environment["STRATA_FFPROBE"] = Join-Path $stageBin "ffprobe.exe"
$start.Environment["GSETTINGS_SCHEMA_DIR"] = Join-Path $Destination "share/glib-2.0/schemas"
$start.Environment["GIO_EXTRA_MODULES"] = Join-Path $Destination "lib/gio/modules"
$start.Environment["GST_PLUGIN_SYSTEM_PATH"] = Join-Path $Destination "lib/gstreamer-1.0"
$start.Environment["GST_PLUGIN_SCANNER"] = $scannerDest
$start.Environment["GDK_PIXBUF_MODULE_FILE"] = $cacheFile
$start.Environment["GDK_PIXBUF_MODULEDIR"] = $loaderDir
$process = [System.Diagnostics.Process]::Start($start)
if (-not $process) { throw "Could not start isolated executable" }
if (-not $process.WaitForExit(15000)) {
    $process.Kill($true)
    throw "Portable Windows version smoke timed out"
}
$output = $process.StandardOutput.ReadToEnd()
$errorOutput = $process.StandardError.ReadToEnd()
if ($process.ExitCode -ne 0 -or $output -notmatch 'strata\s+1\.\d+') {
    throw "Packaged executable failed without MSYS2 PATH. Exit=$($process.ExitCode), output='$output', stderr='$errorOutput'"
}
Write-Host "Portable PE smoke passed: $($output.Trim())"

$start.FileName = Join-Path $Destination 'Start-Strata.exe'
$process = [Diagnostics.Process]::Start($start)
if (-not $process.WaitForExit(15000)) {
    $process.Kill($true)
    throw 'Native portable launcher smoke timed out'
}
$output = $process.StandardOutput.ReadToEnd()
$errorOutput = $process.StandardError.ReadToEnd()
if ($process.ExitCode -ne 0 -or $output -notmatch '^strata\s+1\.\d+' -or $errorOutput) {
    throw "Native portable launcher failed: $output $errorOutput"
}
Write-Host "Native portable launcher smoke passed: $($output.Trim())"

$helperScratch = Join-Path ([IO.Path]::GetTempPath()) ("strata-shell-smoke-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $helperScratch | Out-Null
Copy-Item -LiteralPath $shellHelper -Destination (Join-Path $helperScratch 'strata-shell.exe')
try {
    $helper = [Diagnostics.ProcessStartInfo]::new()
    $helper.FileName = Join-Path $helperScratch 'strata-shell.exe'
    $helper.Arguments = '--version'
    $helper.WorkingDirectory = $helperScratch
    $helper.UseShellExecute = $false
    $helper.CreateNoWindow = $true
    $helper.RedirectStandardOutput = $true
    $helper.RedirectStandardError = $true
    $helper.Environment['PATH'] = "$env:SystemRoot\System32;$env:SystemRoot"
    $helperProcess = [Diagnostics.Process]::Start($helper)
    if (-not $helperProcess.WaitForExit(15000)) {
        $helperProcess.Kill($true)
        throw 'Standalone folder helper smoke timed out'
    }
    $helperOutput = $helperProcess.StandardOutput.ReadToEnd()
    $helperError = $helperProcess.StandardError.ReadToEnd()
    if ($helperProcess.ExitCode -ne 0 -or $helperOutput -notmatch '^strata-shell\s+1\.\d+' -or $helperError) {
        throw "Standalone folder helper requires missing runtime files: $helperOutput $helperError"
    }
    Write-Host "Standalone folder helper smoke passed: $($helperOutput.Trim())"
} finally {
    Remove-Item -LiteralPath (Join-Path $helperScratch 'strata-shell.exe') -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $helperScratch -ErrorAction SilentlyContinue
}

$cmd = [System.Diagnostics.ProcessStartInfo]::new()
$cmd.FileName = Join-Path $env:SystemRoot "System32\cmd.exe"
$cmd.Arguments = '/d /c ""' + (Join-Path $Destination "Start-Strata.cmd") + '" --version"'
$cmd.WorkingDirectory = $Destination
$cmd.UseShellExecute = $false
$cmd.RedirectStandardOutput = $true
$cmd.RedirectStandardError = $true
$cmd.Environment["PATH"] = "$stageBin;$($env:SystemRoot)\System32;$($env:SystemRoot)"
$cmdProcess = [System.Diagnostics.Process]::Start($cmd)
if (-not $cmdProcess) { throw "Could not start portable launcher" }
if (-not $cmdProcess.WaitForExit(15000)) {
    $cmdProcess.Kill($true)
    throw "Portable launcher timed out"
}
$cmdOutput = $cmdProcess.StandardOutput.ReadToEnd()
$cmdError = $cmdProcess.StandardError.ReadToEnd()
if ($cmdProcess.ExitCode -ne 0 -or $cmdOutput -notmatch 'strata\s+1\.\d+') {
    throw "Portable launcher failed. Exit=$($cmdProcess.ExitCode), output='$cmdOutput', stderr='$cmdError'"
}
Write-Host "Portable launcher smoke passed: $($cmdOutput.Trim())"
Write-Host "Bundled runtime dependency imports: $importCount"
Write-Host "Bundled native DLL files: $((Get-ChildItem -LiteralPath $stageBin -Filter '*.dll').Count)"

$manifest = Join-Path $Destination "SHA256SUMS.txt"
Get-ChildItem -LiteralPath $Destination -Recurse -File |
    Where-Object { $_.FullName -ne $manifest } |
    Sort-Object FullName |
    ForEach-Object {
        $relative = $_.FullName.Substring($Destination.Length + 1).Replace("\", "/")
        "$( (Get-FileHash -Algorithm SHA256 -LiteralPath $_.FullName).Hash.ToLowerInvariant() )  $relative"
    } | Set-Content -LiteralPath $manifest -Encoding ASCII
