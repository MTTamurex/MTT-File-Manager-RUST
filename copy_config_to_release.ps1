# Script para copiar arquivos de configuracao para o diretorio de release

$configFile = "virtual_drive_config.json"
$releaseDir = "target\release"

if (Test-Path $configFile) {
    Copy-Item $configFile $releaseDir -Force
    Write-Host "Copiado $configFile para $releaseDir" -ForegroundColor Green
} else {
    Write-Host "Arquivo $configFile nao encontrado na raiz do projeto" -ForegroundColor Yellow
}
