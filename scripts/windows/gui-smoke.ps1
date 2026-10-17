# SPDX-License-Identifier: MIT
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Executable,
    [Parameter(Mandatory = $true)][string]$OutputJson,
    [string]$WorkingDirectory = (Get-Location).Path,
    [int]$TimeoutSeconds = 30
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

Add-Type @'
using System;
using System.Text;
using System.Runtime.InteropServices;

public static class StrataWindowProbe {
    public delegate bool EnumWindowsProc(IntPtr hwnd, IntPtr lparam);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc callback, IntPtr lparam);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hwnd);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowText(IntPtr hwnd, StringBuilder text, int max);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hwnd, uint message, IntPtr wparam, IntPtr lparam);
    public const uint WM_CLOSE = 0x0010;

    public static IntPtr Find(uint targetPid) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((hwnd, _) => {
            uint pid;
            GetWindowThreadProcessId(hwnd, out pid);
            if (pid == targetPid && IsWindowVisible(hwnd)) {
                found = hwnd;
                return false;
            }
            return true;
        }, IntPtr.Zero);
        return found;
    }

    public static string Title(IntPtr hwnd) {
        var text = new StringBuilder(512);
        GetWindowText(hwnd, text, text.Capacity);
        return text.ToString();
    }
}
'@

$exe = (Resolve-Path -LiteralPath $Executable).Path
$work = (Resolve-Path -LiteralPath $WorkingDirectory).Path
$fixture = Join-Path ([System.IO.Path]::GetTempPath()) ("strata-gui-smoke-" + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $fixture -Force | Out-Null
Set-Content -LiteralPath (Join-Path $fixture 'first-paint.txt') -Value 'GUI smoke fixture' -Encoding UTF8

$start = [System.Diagnostics.ProcessStartInfo]::new()
$start.FileName = $exe
$start.Arguments = '"' + $fixture.Replace('"', '""') + '"'
$start.WorkingDirectory = $work
$start.UseShellExecute = $false
$start.RedirectStandardOutput = $false
$start.RedirectStandardError = $false
$process = $null
$watch = [System.Diagnostics.Stopwatch]::StartNew()
try {
    $process = [System.Diagnostics.Process]::Start($start)
    if (-not $process) { throw 'Strata process did not start' }
    $startedMs = $watch.Elapsed.TotalMilliseconds
    $window = [IntPtr]::Zero
    while ($watch.Elapsed.TotalSeconds -lt $TimeoutSeconds) {
        if ($process.HasExited) {
            throw "Strata exited before presenting a window (code=$($process.ExitCode))"
        }
        $window = [StrataWindowProbe]::Find([uint32]$process.Id)
        if ($window -ne [IntPtr]::Zero) { break }
        Start-Sleep -Milliseconds 100
    }
    if ($window -eq [IntPtr]::Zero) {
        throw "Strata did not present a visible window within $TimeoutSeconds seconds"
    }
    $windowMs = $watch.Elapsed.TotalMilliseconds
    $title = [StrataWindowProbe]::Title($window)
    if ([string]::IsNullOrWhiteSpace($title) -or $title -notmatch 'Strata') {
        throw "Unexpected Strata window title: '$title'"
    }
    [StrataWindowProbe]::PostMessage($window, [StrataWindowProbe]::WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
    if (-not $process.WaitForExit(5000)) {
        $process.Kill($true)
        throw 'Strata did not close after WM_CLOSE'
    }
    [pscustomobject]@{
        executable = $exe
        pid = $process.Id
        window_title = $title
        process_start_ms = [math]::Round($startedMs, 2)
        first_visible_window_ms = [math]::Round($windowMs, 2)
        exit_code = $process.ExitCode
    } | ConvertTo-Json | Set-Content -LiteralPath $OutputJson -Encoding UTF8
}
finally {
    if ($process -and -not $process.HasExited) { $process.Kill($true) }
    if (Test-Path -LiteralPath $fixture) { Remove-Item -LiteralPath $fixture -Recurse -Force -ErrorAction SilentlyContinue }
}
