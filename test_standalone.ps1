# Script para testar o executavel standalone (sem depender de assets externos)
Write-Host "========================================" -ForegroundColor Cyan
Write-Host "Teste do Executavel Standalone" -ForegroundColor Cyan
Write-Host "========================================" -ForegroundColor Cyan
Write-Host ""

# Criar diretorio temporario para teste
$tempTestDir = Join-Path $env:TEMP "mtt_standalone_test"
if (Test-Path $tempTestDir) {
    Remove-Item -Path $tempTestDir -Recurse -Force
}
New-Item -ItemType Directory -Path $tempTestDir | Out-Null

Write-Host "1. Copiando executavel para pasta temporaria..." -ForegroundColor Yellow
Copy-Item -Path "target\release\mtt-file-manager.exe" -Destination $tempTestDir

Write-Host "2. Verificando que NAO ha pasta assets no local de teste..." -ForegroundColor Yellow
$assetsExist = Test-Path (Join-Path $tempTestDir "assets")
if ($assetsExist) {
    Write-Host "   ERRO: Pasta assets existe!" -ForegroundColor Red
    exit 1
} else {
    Write-Host "   OK: Pasta assets NAO existe (executavel deve funcionar standalone)" -ForegroundColor Green
}

Write-Host ""
Write-Host "3. Executando o programa a partir da pasta temporaria..." -ForegroundColor Yellow
Write-Host "   Local: $tempTestDir" -ForegroundColor Gray
Write-Host ""
Write-Host "   Os icones SVG e a fonte devem aparecer corretamente!" -ForegroundColor Cyan
Write-Host "   Pressione Ctrl+C para encerrar o teste." -ForegroundColor Gray
Write-Host ""

# Executar o programa
Set-Location $tempTestDir
.\mtt-file-manager.exe

# Cleanup (so executa se o usuario encerrar normalmente)
Write-Host ""
Write-Host "Limpando arquivos de teste..." -ForegroundColor Yellow
Set-Location $PSScriptRoot
Remove-Item -Path $tempTestDir -Recurse -Force
Write-Host "Teste concluido!" -ForegroundColor Green
