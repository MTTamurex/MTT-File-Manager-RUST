# Script para executar o app e capturar logs de debug
Write-Host "Iniciando MTT File Manager com logging de debug..." -ForegroundColor Cyan
Write-Host "Por favor:" -ForegroundColor Yellow
Write-Host "1. Navegue até uma pasta com imagens JPEG (como a foto do aniversário)" -ForegroundColor Yellow
Write-Host "2. Clique em uma imagem para ver detalhes" -ForegroundColor Yellow
Write-Host "3. Clique em um vídeo WebM para ver detalhes" -ForegroundColor Yellow
Write-Host "4. Feche o app quando terminar (Ctrl+C ou Alt+F4)" -ForegroundColor Yellow
Write-Host ""
Write-Host "Os logs aparecerão abaixo:" -ForegroundColor Green
Write-Host "=" * 80 -ForegroundColor Gray

& ".\target\release\mtt-file-manager.exe" 2>&1 | Tee-Object -FilePath "debug_metadata.log"

Write-Host ""
Write-Host "=" * 80 -ForegroundColor Gray
Write-Host "Logs salvos em: debug_metadata.log" -ForegroundColor Cyan
