# SPDX-License-Identifier: MIT
# Developer-only relocatable Windows/UCRT64 runtime bundle. Must run on a
# Windows MSYS2 UCRT64 build runner after strata.exe is linked.
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Executable,
    [Parameter(Mandatory = $true)][string]$UcrtRoot,
    [Parameter(Mandatory = $true)][string]$Destination
)
$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$exe = (Resolve-Path -LiteralPath $Executable).Path
$prefix = (Resolve-Path -LiteralPath $UcrtRoot).Path
$runtimeBin = Join-Path $prefix "bin"
$objdump = Join-Path $runtimeBin "objdump.exe"
if (-not (Test-Path -LiteralPath $objdump)) {
    throw "UCRT64 objdump.exe is required to verify every imported runtime DLL"
}
if (Test-Path -LiteralPath $Destination) {
    Remove-Item -LiteralPath $Destination -Recurse -Force
}
New-Item -ItemType Directory -Path (Join-Path $Destination "bin") -Force | Out-Null
$stageBin = Join-Path $Destination "bin"
Copy-Item -LiteralPath $exe -Destination (Join-Path $stageBin "strata.exe")

# GTK schemas, icon themes, GIO/GdkPixbuf modules, and GStreamer modules are
# loaded dynamically and do not appear in strata.exe's import table. Preserve
# their MSYS2 relative layout; GTK/GLib resolve runtime prefix from their DLL.
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
    $pieces = $relative.Split("/")
    $from = Join-Path $prefix ($pieces -join [System.IO.Path]::DirectorySeparatorChar)
    if (-not (Test-Path -LiteralPath $from)) {
        if ($relative -eq "share/glib-2.0/schemas") {
            throw "Required GTK/GSettings runtime schemas missing: $from"
        }
        continue
    }
    $to = Join-Path $Destination ($pieces -join [System.IO.Path]::DirectorySeparatorChar)
    New-Item -ItemType Directory -Path (Split-Path -Parent $to) -Force | Out-Null
    Copy-Item -LiteralPath $from -Destination $to -Recurse -Force
}

# Recursively walk import tables of both the GUI and every dynamically loaded
# module. No all-DLLs wildcard: it would ship toolchains and unrelated programs.
$visited = [System.Collections.Generic.HashSet[string]]::new(
    [System.StringComparer]::OrdinalIgnoreCase
)
$queued = [System.Collections.Generic.HashSet[string]]::new(
    [System.StringComparer]::OrdinalIgnoreCase
)
$queue = [System.Collections.Generic.Queue[string]]::new()
function Queue-Binary([string]$path) {
    if ($queued.Add($path)) { $queue.Enqueue($path) }
}
Queue-Binary (Join-Path $stageBin "strata.exe")
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
        if ($dll -notmatch '^[\w.+-]+\.dll$') {
            throw "Suspicious DLL import name '$dll' in $binary"
        }
        $importCount++
        if ($dll -ieq "msys-2.0.dll") {
            throw "MSYS2 POSIX-runtime dependency in native Windows package: $binary"
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

$launcher = @'
@echo off
setlocal
set "PATH=%~dp0bin;%SystemRoot%\System32;%SystemRoot%"
set "GSETTINGS_SCHEMA_DIR=%~dp0share\glib-2.0\schemas"
set "GIO_EXTRA_MODULES=%~dp0lib\gio\modules"
set "GST_PLUGIN_SYSTEM_PATH=%~dp0lib\gstreamer-1.0"
set "FONTCONFIG_PATH=%~dp0etc\fonts"
"%~dp0bin\strata.exe" %*
exit /b %ERRORLEVEL%
'@
Set-Content -LiteralPath (Join-Path $Destination "Start-Strata.cmd") -Value $launcher -Encoding ASCII
$warning = @'
STRATA WINDOWS DEVELOPER BUILD — NOT A RELEASE

Start with Start-Strata.cmd, not from a globally installed MSYS2 terminal.
This bundle contains the recursively linked UCRT64 runtime and GTK resources,
but full GTK first-paint and filesystem integration tests remain outstanding.
Rich previews, Recycle Bin/undo, package updates and external providers are
deliberately unavailable until secure Windows-native implementations exist.
Do not install this bundle as a system-level file manager or request UAC.

The runtime DLLs and plugins are open-source third-party components with their
own licenses (see share/licenses); they are not authored by OpenAI or Strata.
'@
Set-Content -LiteralPath (Join-Path $Destination "READ-ME-FIRST.txt") -Value $warning -Encoding UTF8

# Verify no hidden reliance on C:\msys64\ucrt64\bin via runner PATH.
$start = [System.Diagnostics.ProcessStartInfo]::new()
$start.FileName = (Join-Path $stageBin "strata.exe")
$start.Arguments = "--version"
$start.WorkingDirectory = $Destination
$start.UseShellExecute = $false
$start.RedirectStandardOutput = $true
$start.RedirectStandardError = $true
$start.Environment["PATH"] = "$stageBin;$($env:SystemRoot)\System32;$($env:SystemRoot)"
$start.Environment["GSETTINGS_SCHEMA_DIR"] = Join-Path $Destination "share/glib-2.0/schemas"
$start.Environment["GIO_EXTRA_MODULES"] = Join-Path $Destination "lib/gio/modules"
$start.Environment["GST_PLUGIN_SYSTEM_PATH"] = Join-Path $Destination "lib/gstreamer-1.0"
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
Write-Host "Portable runtime smoke passed: $($output.Trim())"
Write-Host "Bundled runtime dependency imports: $importCount"
Write-Host "Bundled native DLL files: $((Get-ChildItem -LiteralPath $stageBin -Filter '*.dll').Count)"

# SHA-256 manifest supports user verification and distinguishes each build.
$manifest = Join-Path $Destination "SHA256SUMS.txt"
Get-ChildItem -LiteralPath $Destination -Recurse -File |
    Where-Object { $_.FullName -ne $manifest } |
    Sort-Object FullName |
    ForEach-Object {
        $relative = $_.FullName.Substring($Destination.Length + 1).Replace("\", "/")
        "$( (Get-FileHash -Algorithm SHA256 -LiteralPath $_.FullName).Hash.ToLowerInvariant() )  $relative"
    } | Set-Content -LiteralPath $manifest -Encoding ASCII
