# SPDX-License-Identifier: MIT
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Executable,
    [Parameter(Mandatory = $true)][string]$UcrtRoot,
    [Parameter(Mandatory = $true)][string]$Destination
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$Destination = [System.IO.Path]::GetFullPath($Destination)

& (Join-Path $PSScriptRoot 'package-portable.ps1') -Executable $Executable -UcrtRoot $UcrtRoot -Destination $Destination -Release
if ($LASTEXITCODE -ne 0) {
    throw 'Portable release staging failed'
}
if (-not (Test-Path -LiteralPath $Destination -PathType Container)) {
    throw "Portable release staging did not create its destination: $Destination"
}

$files = Get-ChildItem -LiteralPath $Destination -Recurse -File |
    Sort-Object FullName |
    ForEach-Object {
        $relative = $_.FullName.Substring($Destination.Length).TrimStart('\', '/')
        [pscustomobject]@{
            path = $relative.Replace('\', '/')
            sha256 = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
            bytes = $_.Length
        }
    }
[pscustomobject]@{
    format = 'strata-windows-portable-v1'
    generated_utc = [DateTime]::UtcNow.ToString('O')
    files = @($files)
} | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $Destination 'release-manifest.json') -Encoding UTF8
