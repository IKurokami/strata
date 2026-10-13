# SPDX-License-Identifier: MIT
[CmdletBinding()]
param(
    [ValidateSet("build", "test", "run", "package")][string]$Action = "build",
    [string]$UcrtRoot = "C:\msys64\ucrt64",
    [string[]]$ApplicationArguments = @()
)
$ErrorActionPreference = "Stop"
$repository = (Resolve-Path (Join-Path $PSScriptRoot "../..")).Path
$prefix = (Resolve-Path -LiteralPath $UcrtRoot).Path
$bin = Join-Path $prefix "bin"
$cargo = Join-Path $bin "cargo.exe"
foreach ($tool in @("cargo.exe", "rustc.exe", "gcc.exe", "pkgconf.exe")) {
    if (-not (Test-Path -LiteralPath (Join-Path $bin $tool))) {
        throw "Missing UCRT64 tool $tool. See docs/windows-development.md."
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
    if ($Action -eq "test") {
        & $cargo test --manifest-path crates/strata-native-io/Cargo.toml --locked --all-targets
        if ($LASTEXITCODE -ne 0) { throw "Native I/O tests failed." }
        & $cargo test --locked --no-default-features --bin strata --no-run
        if ($LASTEXITCODE -ne 0) { throw "Windows application test compilation failed." }
        $inventory = @(& $cargo test --locked --no-default-features --bin strata -- --list)
        if ($LASTEXITCODE -ne 0) { throw "Windows test inventory failed." }
        $cases = @(
            "windows_keeps_saved_action_but_never_executes_a_process_tree",
            "windows_refuses_reparse_action_roots_without_following_them",
            "windows_directory_index_obeys_hidden_attribute_not_dot_prefix",
            "cannot_install_or_launch_untrusted_archive",
            "cancellation_precedes_any_installation",
            "root_confinement_rejects_sibling_and_missing_paths",
            "refuses_linux_trash_metadata_in_windows_recycle_bin",
            "untrusted_parse_refused_without_touching_path",
            "no_external_provider_can_be_discovered_on_windows",
            "rejects_provider_requests_without_executing_helpers",
            "private_preview_helper_rejects_execution"
        )
        foreach ($case in $cases) {
            if (-not ($inventory | Where-Object { $_ -like "*${case}: test" })) {
                throw "Required Windows regression is missing: $case"
            }
            & $cargo test --locked --no-default-features --bin strata $case -- --test-threads=1
            if ($LASTEXITCODE -ne 0) { throw "Windows regression failed: $case" }
        }
    } else {
        & $cargo build --locked --no-default-features --bin strata
        if ($LASTEXITCODE -ne 0) { throw "Windows application build failed." }
        if ($Action -eq "run") {
            & (Join-Path $repository "target/debug/strata.exe") @ApplicationArguments
            if ($LASTEXITCODE -ne 0) { throw "Strata exited with code $LASTEXITCODE." }
        } elseif ($Action -eq "package") {
            $destination = Join-Path $repository ("target/Strata-Windows-" + (Get-Date -Format "yyyyMMdd-HHmmss"))
            & (Join-Path $PSScriptRoot "package-portable.ps1") `
                -Executable (Join-Path $repository "target/debug/strata.exe") `
                -UcrtRoot $prefix -Destination $destination
            Write-Host "Portable Windows build: $destination"
        }
    }
} finally {
    Pop-Location
    foreach ($name in $saved.Keys) {
        [Environment]::SetEnvironmentVariable($name, $saved[$name], "Process")
    }
}
