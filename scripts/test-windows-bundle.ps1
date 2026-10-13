# SPDX-License-Identifier: MIT
# Exercise a staged Windows developer package with no MSYS2 UCRT64 directory
# in PATH. The PE loader must find all direct/transitive imports in the bundle
# or in Windows' standard system locations. This is not a GTK first-paint test.
param(
    [Parameter(Mandatory = $true)]
    [string]$BundleRoot
)

$ErrorActionPreference = 'Stop'
$root = (Resolve-Path -LiteralPath $BundleRoot).Path
$exe = Join-Path $root 'bin\strata.exe'
$launcher = Join-Path $root 'Run-Strata.cmd'
if (!(Test-Path -LiteralPath $exe)) { throw "Missing PE executable: $exe" }
if (!(Test-Path -LiteralPath $launcher)) { throw "Missing launcher: $launcher" }

$windows = $env:SystemRoot
# Deliberately do not inherit C:\msys64\ucrt64\bin or shell tooling. Running
# the program directly verifies the DLL bundle, not the build runner's PATH.
$env:Path = "$root\bin;$windows\System32;$windows"
$env:GSETTINGS_SCHEMA_DIR = Join-Path $root 'share\glib-2.0\schemas'
$env:XDG_DATA_DIRS = Join-Path $root 'share'
$env:GIO_EXTRA_MODULES = Join-Path $root 'lib\gio\modules'

function Check-Version([string]$command, [string]$arguments) {
    $psi = [System.Diagnostics.ProcessStartInfo]::new()
    $psi.FileName = $command
    $psi.Arguments = $arguments
    $psi.UseShellExecute = $false
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.CreateNoWindow = $true
    $process = [System.Diagnostics.Process]::Start($psi)
    if (!$process.WaitForExit(20000)) {
        $process.Kill()
        throw "Strata smoke timed out: $command"
    }
    $stdout = $process.StandardOutput.ReadToEnd()
    $stderr = $process.StandardError.ReadToEnd()
    if ($process.ExitCode -ne 0) {
        throw "Strata exited $($process.ExitCode): $stderr"
    }
    if ($stdout -notmatch '(?i)strata') {
        throw "Unexpected output from $command : $stdout"
    }
    Write-Host "Passed isolated DLL loading: $stdout"
}

Check-Version $exe '--version'
$cmd = Join-Path $windows 'System32\cmd.exe'
Check-Version $cmd ('/d /c "' + '"' + $launcher + '" --version"')
Write-Host "Both the direct Windows PE and the launcher ran without MSYS2 in PATH."
