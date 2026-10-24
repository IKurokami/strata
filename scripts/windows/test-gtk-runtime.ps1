# SPDX-License-Identifier: MIT
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$GtkDll,
    [Parameter(Mandatory = $true)][string]$Output,
    [string]$GstPlayDll,
    [switch]$ExpectFileDropUnsupported,
    [string]$UcrtRoot = 'C:/msys64/ucrt64'
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$Output = [IO.Path]::GetFullPath($Output)
if (Test-Path -LiteralPath $Output) { throw 'Use a fresh probe output directory.' }
New-Item -ItemType Directory -Path $Output | Out-Null
$savedPath = $env:PATH
$savedDebug = $env:G_DEBUG
try {
    $env:PATH = (Join-Path $UcrtRoot 'bin') + ';' + $env:PATH
    $env:G_DEBUG = 'fatal-criticals'
    $flags = @((& (Join-Path $UcrtRoot 'bin/pkgconf.exe') --cflags --libs gtk4 pangocairo) -split ' ' | Where-Object { $_ })
    if ($LASTEXITCODE -ne 0) { throw 'GTK probe build dependencies unavailable.' }
    $source = Join-Path $PSScriptRoot '../../packaging/windows/runtime/ime-lifetime.c'
    $probe = Join-Path $Output 'ime-lifetime.exe'
    & (Join-Path $UcrtRoot 'bin/gcc.exe') -Wall -Wextra -Werror -Wno-unused-parameter $source -o $probe @flags -limm32
    if ($LASTEXITCODE -ne 0) { throw 'IME probe compilation failed.' }
    $dll = Join-Path $Output 'libgtk-4-1.dll'
    Copy-Item -LiteralPath $GtkDll -Destination $dll
    if (-not $GstPlayDll) { $GstPlayDll = Join-Path (Split-Path -Parent $GtkDll) 'libgstplay-1.0-0.dll' }
    Copy-Item -LiteralPath $GstPlayDll -Destination (Join-Path $Output 'libgstplay-1.0-0.dll')
    $streamProbe = Join-Path $PSScriptRoot 'test-file-drop-stream.py'
    $streamArguments = @($Output)
    if ($ExpectFileDropUnsupported) { $streamArguments += '--expect-unsupported' }
    & (Join-Path $UcrtRoot 'bin/python.exe') $streamProbe @streamArguments
    if ($LASTEXITCODE -ne 0) { throw 'Native file-drop stream regression failed.' }
    foreach ($mode in @('dispose', 'detach', 'unrealize', 'repeat-focus', 'reentrant', 'native-font', 'fontconfig')) {
        $stdout = Join-Path $Output "$mode.stdout.log"
        $stderr = Join-Path $Output "$mode.stderr.log"
        $process = Start-Process -FilePath $probe -ArgumentList $mode -PassThru -WindowStyle Hidden -RedirectStandardOutput $stdout -RedirectStandardError $stderr
        if (-not $process.WaitForExit(15000)) {
            $process.Kill()
            throw "IME lifecycle probe timed out: $mode"
        }
        if ($process.ExitCode -ne 0 -or (Get-Item -LiteralPath $stderr).Length -ne 0) {
            throw "IME lifecycle probe failed: $mode; inspect $stderr"
        }
        $text = Get-Content -LiteralPath $stdout -Raw
        if (-not $text.Contains($dll) -or -not $text.Contains("IME lifecycle passed: $mode")) {
            throw "IME probe did not verify the intended DLL and completion: $mode"
        }
        Write-Output "IME lifecycle passed: $mode"
    }
    $fixture = Join-Path $Output 'generated.mp4'
    & (Join-Path $UcrtRoot 'bin/ffmpeg.exe') -hide_banner -loglevel error -f lavfi -i testsrc2=size=128x128:rate=10 -t 2 -c:v libx264 -pix_fmt yuv420p $fixture
    if ($LASTEXITCODE -ne 0) { throw 'Media fixture generation failed.' }
    $probes = Join-Path $PSScriptRoot '../../packaging/media-runtime/probes'
    $play = Join-Path $Output 'play-lifetime.exe'
    $playFlags = @((& (Join-Path $UcrtRoot 'bin/pkgconf.exe') --cflags --libs gstreamer-play-1.0) -split ' ' | Where-Object { $_ })
    if ($LASTEXITCODE -ne 0) { throw 'GstPlay probe build dependencies unavailable.' }
    & (Join-Path $UcrtRoot 'bin/gcc.exe') -Wall -Wextra -Werror (Join-Path $probes 'play-lifetime.c') -o $play @playFlags
    if ($LASTEXITCODE -ne 0) { throw 'GstPlay probe compilation failed.' }
    foreach ($mode in @('main-dispose', 'worker', 'repeat-dispose')) {
        foreach ($trial in 1..10) {
            $stdout = Join-Path $Output "play-$mode-$trial.log"
            $stderr = Join-Path $Output "play-$mode-$trial.stderr.log"
            $process = Start-Process -FilePath $play -ArgumentList @(('"' + $fixture + '"'), $mode) -PassThru -WindowStyle Hidden -RedirectStandardOutput $stdout -RedirectStandardError $stderr
            if (-not $process.WaitForExit(15000)) { $process.Kill(); throw "GstPlay lifecycle timed out: $mode/$trial" }
            if ($process.ExitCode -ne 0 -or (Get-Item -LiteralPath $stderr).Length -ne 0) { throw "GstPlay lifecycle failed: $mode/$trial" }
        }
        Write-Output "GstPlay lifecycle passed: $mode (10 trials)"
    }
    $media = Join-Path $Output 'gtk-lifecycle.exe'
    & (Join-Path $UcrtRoot 'bin/gcc.exe') -Wall -Wextra -Werror (Join-Path $probes 'gtk-lifecycle.c') -o $media @flags
    if ($LASTEXITCODE -ne 0) { throw 'GTK media probe compilation failed.' }
    $stdout = Join-Path $Output 'media.stdout.log'
    $stderr = Join-Path $Output 'media.stderr.log'
    $process = Start-Process -FilePath $media -ArgumentList @(('"' + $fixture + '"'), '50') -PassThru -WindowStyle Hidden -RedirectStandardOutput $stdout -RedirectStandardError $stderr
    $watch = [Diagnostics.Stopwatch]::StartNew()
    while (-not $process.WaitForExit(1000)) {
        if ($watch.Elapsed.TotalSeconds -gt 150) { $process.Kill(); throw 'GTK media lifecycle timed out.' }
    }
    if ($process.ExitCode -ne 0 -or (Get-Item -LiteralPath $stderr).Length -ne 0 -or
        -not (Get-Content -LiteralPath $stdout -Raw).Contains('finalized=50')) {
        throw "GTK media lifecycle failed; inspect $Output"
    }
    Write-Output 'GTK media lifecycle passed: 50 finalized playback/close cycles'
} finally {
    $env:PATH = $savedPath
    $env:G_DEBUG = $savedDebug
}
