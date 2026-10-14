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
$Destination = [System.IO.Path]::GetFullPath($Destination)
$runtimeBin = Join-Path $prefix "bin"
$objdump = Join-Path $runtimeBin "objdump.exe"
if (-not (Test-Path -LiteralPath $objdump)) {
    throw "UCRT64 objdump.exe is required to verify every imported runtime DLL"
}

if (Test-Path -LiteralPath $Destination) {
    throw "Destination already exists. Choose a new staging directory: $Destination"
}
$stageBin = Join-Path $Destination "bin"
New-Item -ItemType Directory -Path $stageBin -Force | Out-Null
Copy-Item -LiteralPath $exe -Destination (Join-Path $stageBin "strata.exe")

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
set "PATH=%~dp0bin;%SystemRoot%\System32;%SystemRoot%"
set "GSETTINGS_SCHEMA_DIR=%~dp0share\glib-2.0\schemas"
set "GIO_EXTRA_MODULES=%~dp0lib\gio\modules"
set "GST_PLUGIN_SYSTEM_PATH=%~dp0lib\gstreamer-1.0"
set "GST_PLUGIN_SCANNER=%~dp0libexec\gstreamer-1.0\gst-plugin-scanner.exe"
set "GDK_PIXBUF_MODULE_FILE=%~dp0lib\gdk-pixbuf-2.0\2.10.0\loaders.cache"
set "GDK_PIXBUF_MODULEDIR=%~dp0lib\gdk-pixbuf-2.0\2.10.0\loaders"
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

# Run both the PE and launcher with only staged DLLs plus Windows system DLLs.
$start = [System.Diagnostics.ProcessStartInfo]::new()
$start.FileName = Join-Path $stageBin "strata.exe"
$start.Arguments = "--version"
$start.WorkingDirectory = $Destination
$start.UseShellExecute = $false
$start.RedirectStandardOutput = $true
$start.RedirectStandardError = $true
$start.Environment["PATH"] = "$stageBin;$($env:SystemRoot)\System32;$($env:SystemRoot)"
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
