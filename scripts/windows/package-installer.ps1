# SPDX-License-Identifier: MIT
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Bundle,
    [Parameter(Mandatory = $true)][string]$Destination,
    [string]$Compiler
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$bundlePath = (Resolve-Path -LiteralPath $Bundle).Path.TrimEnd('\', '/')
$outputPath = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Destination)
if ($outputPath -eq $bundlePath -or $outputPath.StartsWith($bundlePath + '\', [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Installer output must be outside the portable bundle.'
}
$manifest = Get-Content -LiteralPath (Join-Path $bundlePath 'release-manifest.json') -Raw | ConvertFrom-Json
if ($manifest.format -ne 'strata-windows-portable-v1' -or @($manifest.files).Count -eq 0) {
    throw 'Expected a nonempty portable release manifest.'
}
$verified = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
foreach ($entry in $manifest.files) {
    $path = [IO.Path]::GetFullPath((Join-Path $bundlePath $entry.path))
    if (-not $path.StartsWith($bundlePath + '\', [StringComparison]::OrdinalIgnoreCase)) {
        throw "Manifest path escapes bundle: $($entry.path)"
    }
    if ((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash -ine $entry.sha256) {
        throw "Portable bundle checksum mismatch: $($entry.path)"
    }
    if (-not $verified.Add($path)) { throw "Duplicate manifest path: $($entry.path)" }
}
foreach ($file in (Get-ChildItem -LiteralPath $bundlePath -Recurse -Force)) {
    if ($file.Attributes -band [IO.FileAttributes]::ReparsePoint) {
        throw "Portable bundle contains a reparse point: $($file.FullName)"
    }
    if (-not $file.PSIsContainer -and $file.FullName -ne (Join-Path $bundlePath 'release-manifest.json') -and -not $verified.Contains($file.FullName)) {
        throw "Portable bundle contains an unverified file: $($file.FullName)"
    }
}
foreach ($required in @('bin/strata.exe', 'Start-Strata.cmd')) {
    if (-not ($manifest.files | Where-Object { $_.path -eq $required })) {
        throw "Portable manifest is missing $required"
    }
}
if (-not $Compiler) {
    $candidates = @(
        (Join-Path ${env:ProgramFiles(x86)} 'Inno Setup 6\ISCC.exe'),
        (Join-Path $env:LOCALAPPDATA 'Programs\Inno Setup 6\ISCC.exe')
    )
    $Compiler = $candidates | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
}
if (-not $Compiler) { throw 'Install Inno Setup 6 or supply -Compiler pointing to ISCC.exe.' }
$repository = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$versionLine = Get-Content -LiteralPath (Join-Path $repository 'Cargo.toml') |
    Where-Object { $_ -match '^version\s*=\s*"([0-9]+\.[0-9]+\.[0-9]+)"' } | Select-Object -First 1
if (-not $versionLine -or $versionLine -notmatch '"([0-9]+\.[0-9]+\.[0-9]+)"') { throw 'Cannot determine application version.' }
$version = $Matches[1]
New-Item -ItemType Directory -Path $outputPath -Force | Out-Null
& $Compiler "/DBundleDir=$bundlePath" "/DAppVersion=$version" "/DOutputDir=$outputPath" (Join-Path $repository 'packaging/windows/strata.iss')
if ($LASTEXITCODE -ne 0) { throw "Installer compiler failed: $LASTEXITCODE" }
$installer = Join-Path $outputPath "Strata-$version-windows-x64-setup.exe"
if (-not (Test-Path -LiteralPath $installer)) { throw 'Installer output is missing.' }
Write-Host "Installer: $installer"
Get-FileHash -LiteralPath $installer -Algorithm SHA256
