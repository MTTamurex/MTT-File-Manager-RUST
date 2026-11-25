<#
.SYNOPSIS
    Capture how MTT File Manager frames reach the screen, using PresentMon.

.DESCRIPTION
    App-side frame timing (MTT_TRACE_FRAME_GAPS) only sees when a frame leaves
    the app. PresentMon records, per presented frame, the presentation mode
    (independent flip vs. DWM composition vs. GDI copy), when it was actually
    displayed, and dropped frames, for the app and for DWM.

    Must run in an elevated PowerShell (ETW tracing). Start MTT File Manager
    normally (not elevated) with the renderer/GPU you want, prepare the scenario
    (e.g. play a video), then run this script: it waits -Delay seconds so you can
    start scrolling, records -Seconds seconds and writes one CSV per process.

.EXAMPLE
    .\scripts\present-capture.ps1 -Name vulkan_igpu_video
#>
param(
    [Parameter(Mandatory)][string]$Name,
    [int]$Seconds = 15,
    [int]$Delay = 3,
    [string]$PresentMon = (Join-Path $PSScriptRoot '..\target\tools\PresentMon-2.6.0-x64.exe'),
    [string]$OutDir = (Join-Path $PSScriptRoot '..\target\present-capture')
)

$principal = [Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'PresentMon needs an elevated PowerShell (Run as administrator).'
}
if (-not (Test-Path $PresentMon)) { throw "PresentMon not found: $PresentMon" }
if (-not (Get-Process -Name 'mtt-file-manager' -ErrorAction SilentlyContinue)) {
    throw 'Start MTT File Manager first (normally, not as administrator).'
}

$target = Join-Path $OutDir $Name
New-Item -ItemType Directory -Force $target | Out-Null
Write-Host "Recording $Seconds s after a $Delay s delay: start scrolling now." -ForegroundColor Cyan
& $PresentMon `
    --process_name mtt-file-manager.exe `
    --process_name dwm.exe `
    --multi_csv `
    --output_file (Join-Path $target 'present.csv') `
    --delay $Delay --timed $Seconds --terminate_after_timed `
    --track_gpu_video --track_hybrid_present `
    --stop_existing_session --no_console_stats
Write-Host "Done: $target" -ForegroundColor Green
Get-ChildItem $target -Filter *.csv | ForEach-Object { "  $($_.Name)  $([math]::Round($_.Length / 1KB)) KB" }
