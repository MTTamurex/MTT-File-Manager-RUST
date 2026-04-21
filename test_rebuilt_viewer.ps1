# Test the rebuilt image viewer DIRECTLY (bypassing the file manager parent process).
# Use this to determine whether flicker comes from the viewer code or from the parent app.

param(
    [string]$ImagePath
)

if (-not $ImagePath) {
    Write-Host "Usage: .\test_rebuilt_viewer.ps1 <image_path>"
    exit 1
}

if (-not (Test-Path $ImagePath)) {
    Write-Host "Error: File not found: $ImagePath"
    exit 1
}

Write-Host "Building..."
cargo build --release --quiet

if ($LASTEXITCODE -ne 0) {
    Write-Host "Build failed!"
    exit 1
}

Write-Host "Launching REBUILT viewer DIRECTLY (no parent app):"
Write-Host "  $ImagePath"
Write-Host ""

& ".\target\release\mtt-file-manager.exe" --image-viewer $ImagePath
