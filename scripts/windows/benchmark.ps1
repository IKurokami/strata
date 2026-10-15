# SPDX-License-Identifier: MIT
# Windows Performance Benchmark Script
[CmdletBinding()]
param(
    [string]$UcrtRoot = "C:\msys64\ucrt64"
)
$ErrorActionPreference = "Stop"
$repository = (Resolve-Path (Join-Path $PSScriptRoot "../..")).Path
$prefix = (Resolve-Path -LiteralPath $UcrtRoot).Path
$bin = Join-Path $prefix "bin"

$env:PATH = "$bin;$env:PATH"
$manifest = Join-Path $repository "crates/strata-native-io/Cargo.toml"

Write-Host "Running Strata Windows Native Performance Benchmark..." -ForegroundColor Cyan
& cargo run --manifest-path $manifest --example windows_benchmark
if ($LASTEXITCODE -ne 0) {
    throw "Benchmark run failed with code $LASTEXITCODE"
}
