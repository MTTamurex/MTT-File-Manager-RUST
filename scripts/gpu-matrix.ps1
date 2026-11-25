<#
.SYNOPSIS
    Launch MTT File Manager once per presentation variant and keep each log.

.DESCRIPTION
    For each variant the app is started with the matching MTT_* environment
    variables and MTT_TRACE_FRAME_GAPS=1. Scroll a folder full of thumbnails for
    ~10 seconds (optionally with a video playing), then close the app; the next
    variant starts. Logs land in -OutDir, one file per variant, and a summary of
    the "[PERF] frame timing" lines is printed at the end.

    The renderer (DX12/Vulkan/OpenGL) and the GPU preference still come from the
    app settings: change them there between runs of this script.

.EXAMPLE
    .\scripts\gpu-matrix.ps1
    .\scripts\gpu-matrix.ps1 -Variants baseline,noredir_lat1 -Exe .\target\release\mtt-file-manager.exe
#>
param(
    [string]$Exe = (Join-Path $PSScriptRoot '..\target\release\mtt-file-manager.exe'),
    [string]$OutDir = (Join-Path $PSScriptRoot '..\target\gpu-matrix'),
    [string[]]$Variants
)

$all = [ordered]@{
    baseline        = @{}
    root_alpha      = @{ MTT_WGPU_ROOT_ALPHA = '1' }
    lat1            = @{ MTT_WGPU_LATENCY = '1' }
    opaque          = @{ MTT_MAIN_TRANSPARENT = '0' }
    noredir         = @{ MTT_DX12_SWAPCHAIN = 'visual_noredir' }
    noredir_lat1    = @{ MTT_DX12_SWAPCHAIN = 'visual_noredir'; MTT_WGPU_LATENCY = '1' }
    hwnd_opaque     = @{ MTT_DX12_SWAPCHAIN = 'hwnd'; MTT_MAIN_TRANSPARENT = '0' }
    hwnd_opaque_lat1 = @{ MTT_DX12_SWAPCHAIN = 'hwnd'; MTT_MAIN_TRANSPARENT = '0'; MTT_WGPU_LATENCY = '1' }
    novsync         = @{ MTT_WGPU_PRESENT = 'auto_novsync' }
    memperf         = @{ MTT_WGPU_MEMHINT = 'performance' }
    # Detail-panel video (play a video in the preview panel while scrolling). Popup host is the default.
    video_child     = @{ MTT_MPV_PREVIEW_HOST = 'child' }
    video_noflip    = @{ MTT_MPV_PREVIEW_HOST = 'child'; MTT_MPV_PREVIEW_FLIP = 'no' }
    video_adapter   = @{ MTT_MPV_PREVIEW_ADAPTER = 'render' }
}
$knobs = 'MTT_DX12_SWAPCHAIN', 'MTT_MAIN_TRANSPARENT', 'MTT_WGPU_LATENCY', 'MTT_WGPU_PRESENT', 'MTT_WGPU_MEMHINT', 'MTT_WGPU_ROOT_ALPHA', 'MTT_MPV_PREVIEW_HOST', 'MTT_MPV_PREVIEW_FLIP', 'MTT_MPV_PREVIEW_ADAPTER'

if (-not (Test-Path $Exe)) { throw "Executable not found: $Exe (build with: cargo build --release)" }
$exeDir = Split-Path -Parent (Resolve-Path $Exe)
foreach ($dll in 'libmpv-2.dll', 'pdfium.dll') {
    if (-not (Test-Path (Join-Path $exeDir $dll))) {
        throw "$dll is missing next to $Exe; copy it from another build's target\release folder"
    }
}
New-Item -ItemType Directory -Force $OutDir | Out-Null
$selected = if ($Variants) { $Variants } else { @($all.Keys) }

foreach ($name in $selected) {
    if (-not $all.Contains($name)) { Write-Warning "Unknown variant '$name'"; continue }
    foreach ($knob in $knobs) { Remove-Item "Env:$knob" -ErrorAction SilentlyContinue }
    foreach ($entry in $all[$name].GetEnumerator()) { Set-Item "Env:$($entry.Key)" $entry.Value }
    $env:MTT_TRACE_FRAME_GAPS = '1'
    $log = Join-Path $OutDir "$name.log"
    Write-Host "`n== $name : scroll for ~10 s, then close the app" -ForegroundColor Cyan
    $process = Start-Process -FilePath $Exe -RedirectStandardError $log -Wait -NoNewWindow -PassThru
    if ($process.ExitCode -ne 0) {
        Write-Warning ("$name exited with code 0x{0:X8}" -f $process.ExitCode)
    }
    if ((Get-Item $log).Length -eq 0) {
        Write-Warning "$name wrote no log: did the app open? (an already running MTT instance takes over the launch)"
    }
}
foreach ($knob in $knobs + 'MTT_TRACE_FRAME_GAPS') { Remove-Item "Env:$knob" -ErrorAction SilentlyContinue }

Write-Host "`n== Summary (frame timing lines per variant)" -ForegroundColor Cyan
foreach ($name in $selected) {
    $log = Join-Path $OutDir "$name.log"
    if (-not (Test-Path $log)) { continue }
    Write-Host "`n[$name]" -ForegroundColor Yellow
    Select-String -Path $log -Pattern 'GPU present tuning|display adapter|MpvPreview\] host tuning|\[PERF\] frame timing' |
        ForEach-Object { $_.Line -replace '^.*?\] ', '' }
}
