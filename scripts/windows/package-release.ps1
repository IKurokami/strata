# SPDX-License-Identifier: MIT
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Executable,
    [Parameter(Mandatory = $true)][string]$UcrtRoot,
    [Parameter(Mandatory = $true)][string]$Destination
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

& (Join-Path $PSScriptRoot 'package-portable.ps1') -Executable $Executable -UcrtRoot $UcrtRoot -Destination $Destination -Release
if ($LASTEXITCODE -ne 0) {
    throw 'Portable release staging failed'
}

$files = Get-ChildItem -LiteralPath $Destination -Recurse -File |
    Sort-Object FullName |
    ForEach-Object {
        [pscustomobject]@{
            path = [System.IO.Path]::GetRelativePath($Destination, $_.FullName).Replace('\', '/')
            sha256 = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
            bytes = $_.Length
        }
    }
[pscustomobject]@{
    format = 'strata-windows-portable-v1'
    generated_utc = [DateTime]::UtcNow.ToString('O')
    files = @($files)
} | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $Destination 'release-manifest.json') -Encoding UTF8
