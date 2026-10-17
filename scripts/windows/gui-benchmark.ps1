# SPDX-License-Identifier: MIT
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Executable,
    [Parameter(Mandatory = $true)][string]$OutputJson,
    [string]$WorkingDirectory = (Get-Location).Path,
    [int]$Samples = 10
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ($Samples -lt 3 -or $Samples -gt 100) {
    throw 'Samples must be between 3 and 100'
}

$rows = [System.Collections.Generic.List[object]]::new()
for ($index = 1; $index -le $Samples; $index++) {
    $sample = Join-Path ([System.IO.Path]::GetTempPath()) ("strata-gui-benchmark-" + [Guid]::NewGuid().ToString('N') + '.json')
    try {
        $arguments = @(
            '-NoProfile',
            '-ExecutionPolicy', 'Bypass',
            '-File', (Join-Path $PSScriptRoot 'gui-smoke.ps1'),
            '-Executable', $Executable,
            '-WorkingDirectory', $WorkingDirectory,
            '-OutputJson', $sample
        )
        & pwsh @arguments
        $rows.Add((Get-Content -LiteralPath $sample -Raw | ConvertFrom-Json))
    }
    finally {
        if (Test-Path -LiteralPath $sample) {
            Remove-Item -LiteralPath $sample -Force -ErrorAction SilentlyContinue
        }
    }
}

$ordered = @($rows | ForEach-Object { [double]$_.first_visible_window_ms } | Sort-Object)
function Percentile([double[]]$values, [double]$fraction) {
    $position = ($values.Count - 1) * $fraction
    $lower = [math]::Floor($position)
    $upper = [math]::Ceiling($position)
    if ($lower -eq $upper) { return $values[$lower] }
    return $values[$lower] + ($values[$upper] - $values[$lower]) * ($position - $lower)
}

[pscustomobject]@{
    executable = (Resolve-Path -LiteralPath $Executable).Path
    samples = $ordered.Count
    first_visible_window_ms = @{
        min = [math]::Round($ordered[0], 2)
        p50 = [math]::Round((Percentile $ordered 0.50), 2)
        p95 = [math]::Round((Percentile $ordered 0.95), 2)
        max = [math]::Round($ordered[-1], 2)
    }
    samples_detail = $rows
} | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $OutputJson -Encoding UTF8
