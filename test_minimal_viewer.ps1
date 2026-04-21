# Test the minimal image viewer without the complex startup machinery
# Usage: .\test_minimal_viewer.ps1 [image_path]

param(
    [string]$ImagePath
)

if (-not $ImagePath) {
    Write-Host "Usage: .\test_minimal_viewer.ps1 <image_path>"
    Write-Host "Example: .\test_minimal_viewer.ps1 'C:\path\to\image.jpg'"
    exit 1
}

if (-not (Test-Path $ImagePath)) {
    Write-Host "Error: File not found: $ImagePath"
    exit 1
}

Write-Host "Building MTT File Manager..."
cargo build --release --quiet

if ($LASTEXITCODE -ne 0) {
    Write-Host "Build failed!"
    exit 1
}

$executable = ".\target\release\mtt-file-manager.exe"

Write-Host "Launching MINIMAL viewer with: $ImagePath"
Write-Host "(This viewer has NO cache, NO prefetch, NO sequence, NO filmstrip)"
Write-Host "Just: open window -> load image -> display"
Write-Host ""

& $executable --image-viewer-minimal $ImagePath
