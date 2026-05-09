# Script to run the app and capture debug logs
Write-Host "MTT File Manager - Diagnostic Console" -ForegroundColor Cyan
Write-Host ("=" * 80) -ForegroundColor Gray
Write-Host "Logs will be shown in real time and saved to: debug_metadata.log" -ForegroundColor Green
Write-Host ("=" * 80) -ForegroundColor Gray

& ".\target\release\mtt-file-manager.exe" 2>&1 | Tee-Object -FilePath "debug_metadata.log"

Write-Host ""
Write-Host ("=" * 80) -ForegroundColor Gray
Write-Host "Logs saved to: debug_metadata.log" -ForegroundColor Cyan
