param(
    [Parameter(Mandatory = $true)][string]$Dll,
    [Parameter(Mandatory = $true)][string]$WorkDir,
    [string]$Ffmpeg = 'ffmpeg',
    [ValidateRange(1, 10000)][int]$Repeat = 25
)

$ErrorActionPreference = 'Stop'
$manifest = Join-Path $PSScriptRoot '..\Cargo.toml'
$exe = Join-Path $PSScriptRoot '..\target\release\mpv-thumbnail-poc.exe'
$Dll = (Resolve-Path -LiteralPath $Dll).Path
if (Test-Path -LiteralPath $WorkDir) {
    throw 'WorkDir must be a new directory; no existing files will be overwritten.'
}
$null = New-Item -ItemType Directory -Path $WorkDir
$WorkDir = (Resolve-Path -LiteralPath $WorkDir).Path

& cargo build --release --locked --manifest-path $manifest
if ($LASTEXITCODE -ne 0) { throw 'POC build failed' }

function New-Fixture([string]$Name, [string[]]$Encoder, [string]$Source = 'testsrc2=size=640x360:rate=24:duration=3') {
    $output = Join-Path $WorkDir $Name
    & $Ffmpeg -hide_banner -loglevel error -n -f lavfi -i $Source @Encoder $output
    if ($LASTEXITCODE -ne 0) { throw "Fixture generation failed: $Name" }
    return $output
}

function Invoke-Poc([string]$Name, [string[]]$Arguments, [bool]$ExpectedSuccess = $true) {
    $report = Join-Path $WorkDir "$Name.jsonl"
    & $exe --dll $Dll --check-release @Arguments | Tee-Object -FilePath $report
    $code = $LASTEXITCODE
    if (($ExpectedSuccess -and $code -ne 0) -or (!$ExpectedSuccess -and $code -ne 1)) {
        throw "Unexpected exit code $code for test $Name"
    }
    return $report
}

function Assert-Report([string]$Name, [int]$Successes, [int]$Failures) {
    $rows = @(Get-Content -LiteralPath (Join-Path $WorkDir "$Name.jsonl") | ForEach-Object { $_ | ConvertFrom-Json })
    $summary = $rows[-1].summary
    if ($summary.successful -ne $Successes -or $summary.failed -ne $Failures) {
        throw "Unexpected success/failure counts for test $Name"
    }
    foreach ($row in $rows) {
        if ($row.ok -eq $true) {
            if ($row.capture.video_out -ne 'null' -or $row.capture.hwdec -ne 'no') {
                throw "Unexpected video output or hardware decoder in test $Name"
            }
            if ($row.capture.width -gt 256 -or $row.capture.height -gt 256) {
                throw "Thumbnail exceeds the requested dimensions in test $Name"
            }
            if ($row.capture.strategy -eq 'adaptive-v2') {
                if ($row.capture.attempts -lt 1 -or $row.capture.attempts -gt 3) {
                    throw "Invalid adaptive attempt count in test $Name"
                }
                if ($row.capture.raw_width -gt 256 -or $row.capture.raw_height -gt 256) {
                    throw "Native capture was not reduced before copying in test $Name"
                }
            }
        }
    }
}

# FFmpeg is a TEST FIXTURE GENERATOR ONLY, never invoked by the POC.
$h264 = New-Fixture 'h264.mp4' @('-c:v', 'libx264', '-pix_fmt', 'yuv420p')
$hevc = New-Fixture 'hevc.mkv' @('-c:v', 'libx265', '-preset', 'ultrafast', '-x265-params', 'log-level=error')
$vp9 = New-Fixture 'vp9.webm' @('-c:v', 'libvpx-vp9', '-deadline', 'realtime', '-cpu-used', '8')
$av1 = New-Fixture 'av1.mkv' @('-c:v', 'libaom-av1', '-cpu-used', '8', '-crf', '45', '-row-mt', '1')
$vertical = New-Fixture 'vertical.mp4' @('-c:v', 'libx264') 'testsrc2=size=180x320:rate=12:duration=1'
$short = New-Fixture 'short.mp4' @('-c:v', 'libx264') 'testsrc2=size=160x90:rate=24:duration=0.2'
$hdr = New-Fixture 'hdr.mkv' @('-c:v', 'libx265', '-preset', 'ultrafast', '-pix_fmt', 'yuv420p10le', '-color_primaries', 'bt2020', '-color_trc', 'smpte2084', '-colorspace', 'bt2020nc', '-x265-params', 'log-level=error') 'testsrc2=size=320x180:rate=12:duration=1'
$uhd = New-Fixture 'uhd.mp4' @('-c:v', 'libx264', '-preset', 'ultrafast') 'testsrc2=size=3840x2160:rate=2:duration=1'
$rotated = Join-Path $WorkDir 'rotated.mp4'
& $Ffmpeg -hide_banner -loglevel error -n -display_rotation:v:0 90 -i $h264 -c copy $rotated
if ($LASTEXITCODE -ne 0) { throw 'Rotation fixture generation failed' }
$unicode = Join-Path $WorkDir 'video com espaços 日本語.mp4'
[IO.File]::Copy($h264, $unicode, $false)
$corrupt = Join-Path $WorkDir 'corrupt.mp4'
[IO.File]::WriteAllBytes($corrupt, [byte[]](0..63))
$truncated = Join-Path $WorkDir 'truncated.mp4'
[IO.File]::WriteAllBytes($truncated, [IO.File]::ReadAllBytes($h264)[0..127])
$audio = New-Fixture 'audio.m4a' @('-c:a', 'aac') 'sine=frequency=440:duration=1'

$null = Invoke-Poc 'runtime' @('--inspect')
$valid = @($h264, $hevc, $vp9, $av1, $vertical, $short, $hdr, $uhd, $rotated, $unicode)
$null = Invoke-Poc 'formats' $valid
$null = Invoke-Poc 'reuse-mixed' (@('--reuse', '--repeat', "$Repeat") + $valid)
$null = Invoke-Poc 'reuse' (@('--reuse', '--repeat', "$Repeat") + @($h264, $hevc, $vp9, $av1))
$null = Invoke-Poc 'fresh' (@('--repeat', "$Repeat") + @($h264, $hevc, $vp9, $av1))
$null = Invoke-Poc 'invalid' @($corrupt, $truncated, $audio) $false
$null = Invoke-Poc 'recovery' @('--reuse', $corrupt, $h264, $audio, $vp9) $false
$null = Invoke-Poc 'past-end' @('--at', '9', $h264) $false
$null = Invoke-Poc 'near-end' @('--at', '2.95', $h264)
Assert-Report 'formats' 10 0
Assert-Report 'reuse-mixed' (10 * $Repeat) 0
Assert-Report 'reuse' (4 * $Repeat) 0
Assert-Report 'fresh' (4 * $Repeat) 0
Assert-Report 'invalid' 0 3
Assert-Report 'recovery' 2 2
Assert-Report 'past-end' 0 1
Assert-Report 'near-end' 1 0
$formatRows = @(Get-Content -LiteralPath (Join-Path $WorkDir 'formats.jsonl') | ForEach-Object { $_ | ConvertFrom-Json } | Where-Object { $_.ok -eq $true })
if ($formatRows[4].capture.width -ne 144 -or $formatRows[4].capture.height -ne 256) {
    throw 'Vertical video has incorrect thumbnail dimensions'
}
if ($formatRows[8].capture.width -ne 144 -or $formatRows[8].capture.height -ne 256) {
    throw 'Display rotation was not applied to the thumbnail'
}

# Save explicit inspection outputs, never as part of the normal extraction path.
foreach ($name in @('h264', 'vertical', 'rotated', 'hdr')) {
    $file = if ($name -eq 'hdr') { $hdr } elseif ($name -eq 'rotated') { $rotated } elseif ($name -eq 'vertical') { $vertical } else { $h264 }
    $null = Invoke-Poc "image-$name" @('--output', (Join-Path $WorkDir "$name.png"), $file)
}
$null = Invoke-Poc 'seek-early' @('--at', '0', '--output', (Join-Path $WorkDir 'early.png'), $h264)
$null = Invoke-Poc 'seek-late' @('--at', '2', '--output', (Join-Path $WorkDir 'late.png'), $h264)
if ((Get-FileHash (Join-Path $WorkDir 'early.png')).Hash -eq (Get-FileHash (Join-Path $WorkDir 'late.png')).Hash) {
    throw 'Different seek timestamps returned identical thumbnails'
}
$null = Invoke-Poc 'no-overwrite' @('--output', (Join-Path $WorkDir 'early.png'), $h264) $false

# Confirm native handles do not keep source files locked after the process exits.
foreach ($file in $valid + @($corrupt, $truncated, $audio)) {
    $stream = [IO.File]::Open($file, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
    $stream.Dispose()
}
Write-Output 'Smoke tests passed. Inspect the JSONL reports and PNGs in WorkDir.'
