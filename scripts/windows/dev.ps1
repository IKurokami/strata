# SPDX-License-Identifier: MIT
[CmdletBinding()]
param(
    [ValidateSet("build", "test", "run", "package")][string]$Action = "build",
    [string]$UcrtRoot = "C:\msys64\ucrt64",
    [string[]]$ApplicationArguments = @(),
    [string]$GtkRuntime = (Join-Path $PSScriptRoot '../../target/windows-gtk-runtime')
)
$ErrorActionPreference = "Stop"
$repository = (Resolve-Path (Join-Path $PSScriptRoot "../..")).Path
$prefix = (Resolve-Path -LiteralPath $UcrtRoot).Path
$bin = Join-Path $prefix "bin"
$cargo = Join-Path $bin "cargo.exe"
function Stage-PrivateRuntime([string]$Destination) {
    $manifest = Get-Content -LiteralPath (Join-Path $GtkRuntime 'runtime.json') -Raw | ConvertFrom-Json
    $kit = Join-Path $repository 'packaging/windows/runtime'
    if ($manifest.format -ne 'strata-private-gtk-v1' -or $manifest.version -ne '4.24.1' -or
        $manifest.gstreamer_version -ne '1.28.8' -or
        $manifest.patch_sha256 -ine (Get-FileHash (Join-Path $kit 'gtk-4.24.1-ime-lifetime.patch')).Hash -or
        $manifest.gstreamer_patch_sha256 -ine (Get-FileHash (Join-Path $kit 'gstreamer-1.28.8-defer-worker-finalize.patch')).Hash) {
        throw 'Private runtime provenance is stale. Rebuild scripts/windows/build-gtk-runtime.ps1.'
    }
    New-Item -ItemType Directory -Path $Destination -Force | Out-Null
    foreach ($entry in @(@('libgtk-4-1.dll', $manifest.dll_sha256), @('libgstplay-1.0-0.dll', $manifest.gstplay_dll_sha256))) {
        $source = Join-Path $GtkRuntime $entry[0]
        if ((Get-FileHash -LiteralPath $source).Hash -ine $entry[1]) { throw "Private runtime checksum mismatch: $($entry[0])" }
        Copy-Item -LiteralPath $source -Destination (Join-Path $Destination $entry[0]) -Force
    }
}
if ($Action -ne 'build' -and -not (Test-Path -LiteralPath (Join-Path $GtkRuntime 'runtime.json'))) {
    throw 'Build the private runtime with scripts/windows/build-gtk-runtime.ps1 before running, testing, or packaging Strata.'
}
foreach ($tool in @("cargo.exe", "rustc.exe", "gcc.exe", "pkgconf.exe")) {
    if (-not (Test-Path -LiteralPath (Join-Path $bin $tool))) {
        throw "Missing UCRT64 tool $tool. See docs/windows-development.md."
    }
}
$gstInspect = Join-Path $bin "gst-inspect-1.0.exe"
if (-not (Test-Path -LiteralPath $gstInspect)) {
    throw "Missing GStreamer inspection tool. Install mingw-w64-ucrt-x86_64-gstreamer."
}
foreach ($tool in @("ffmpeg.exe", "ffprobe.exe")) {
    if (-not (Test-Path -LiteralPath (Join-Path $bin $tool))) {
        throw "Missing media decoder $tool. Install mingw-w64-ucrt-x86_64-ffmpeg."
    }
}
$saved = @{}
$variables = @{
    PATH = "$bin;$env:PATH"
    RUSTC = (Join-Path $bin "rustc.exe")
    PKG_CONFIG_PATH = (Join-Path $prefix "lib/pkgconfig")
    PKG_CONFIG = (Join-Path $bin "pkgconf.exe")
    MSYSTEM = "UCRT64"
    MINGW_PREFIX = $prefix
}
if (-not $env:DBUS_SESSION_BUS_ADDRESS) {
    $variables['DBUS_SESSION_BUS_ADDRESS'] = 'disabled:'
}
foreach ($name in $variables.Keys) {
    $saved[$name] = [Environment]::GetEnvironmentVariable($name, "Process")
    [Environment]::SetEnvironmentVariable($name, $variables[$name], "Process")
}
Push-Location $repository
try {
    $compiler = (& (Join-Path $bin "rustc.exe") -vV) -join "`n"
    if ($LASTEXITCODE -ne 0 -or $compiler -notmatch 'host: x86_64-pc-windows-gnu') {
        throw "The Windows GUI requires the native UCRT64 GNU toolchain."
    }
    & $gstInspect autoaudiosink *> $null
    if ($LASTEXITCODE -ne 0) {
        throw "GStreamer autoaudiosink is unavailable. Install mingw-w64-ucrt-x86_64-gst-plugins-good."
    }
    if ($Action -eq "test") {
        # Preview regression tests launch the installed-style executable in a
        # contained Job Object; build that executable before compiling the test
        # harness so a stale or missing target/debug/strata.exe cannot mask the
        # helper operation under test.
        & $cargo build --locked --no-default-features --features rar --bins
        if ($LASTEXITCODE -ne 0) { throw "Windows application helper build failed." }
        Stage-PrivateRuntime (Join-Path $repository 'target/debug')
        & $cargo test --manifest-path crates/strata-native-io/Cargo.toml --locked --all-targets
        if ($LASTEXITCODE -ne 0) { throw "Native I/O tests failed." }
        & $cargo test --manifest-path crates/strata-win-shell/Cargo.toml --locked --all-targets
        if ($LASTEXITCODE -ne 0) { throw "Windows shell integration tests failed." }
        & $cargo test --locked --no-default-features --features rar --bin strata --no-run
        if ($LASTEXITCODE -ne 0) { throw "Windows application test compilation failed." }
        Stage-PrivateRuntime (Join-Path $repository 'target/debug/deps')
        $inventory = @(& $cargo test --locked --no-default-features --features rar --bin strata -- --list)
        if ($LASTEXITCODE -ne 0) { throw "Windows test inventory failed." }
        $cases = @(Get-Content -LiteralPath (Join-Path $PSScriptRoot "regression-tests.txt") | Where-Object { $_.Trim() -and -not $_.StartsWith("#") })
        if ($cases.Count -eq 0) { throw "Windows regression inventory is empty." }
        foreach ($case in $cases) {
            if (-not ($inventory | Where-Object { $_ -like "*${case}: test" })) {
                throw "Required Windows regression is missing: $case"
            }
            & $cargo test --locked --no-default-features --features rar --bin strata $case -- --test-threads=1
            if ($LASTEXITCODE -ne 0) { throw "Windows regression failed: $case" }
        }
    } else {
        & $cargo build --locked --no-default-features --features rar --bins
        if ($LASTEXITCODE -ne 0) { throw "Windows application build failed." }
        if (Test-Path -LiteralPath (Join-Path $GtkRuntime 'runtime.json')) {
            Stage-PrivateRuntime (Join-Path $repository 'target/debug')
        }
        if ($Action -eq "run") {
            & (Join-Path $repository "target/debug/strata.exe") @ApplicationArguments
            if ($LASTEXITCODE -ne 0) { throw "Strata exited with code $LASTEXITCODE." }
        } elseif ($Action -eq "package") {
            $destination = Join-Path $repository ("target/Strata-Windows-" + (Get-Date -Format "yyyyMMdd-HHmmss"))
            & (Join-Path $PSScriptRoot "package-release.ps1") `
                -Executable (Join-Path $repository "target/debug/strata.exe") `
                -UcrtRoot $prefix -Destination $destination -GtkRuntime $GtkRuntime
            Write-Host "Portable Windows build: $destination"
        }
    }
} finally {
    Pop-Location
    foreach ($name in $saved.Keys) {
        [Environment]::SetEnvironmentVariable($name, $saved[$name], "Process")
    }
}
