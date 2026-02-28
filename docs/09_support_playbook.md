# Playbook de Suporte - MTT File Manager

## Objetivo do Documento
Este documento fornece um playbook para triagem e resolução de problemas do MTT File Manager, incluindo checklists, perguntas padrão e procedimentos de suporte.

## Checklist de Triagem Inicial

### 1. Informações Básicas
- [ ] **Versão do aplicativo**: Qual release ou commit? (`cargo pkgid` ou verificar Cargo.toml)
- [ ] **Sistema operacional**: Windows 10/11? Build? (`winver`)
- [ ] **Arquitetura**: x64? ARM64?
- [ ] **Hardware**: CPU, RAM, GPU relevante
- [ ] **Localização dos arquivos**: Local, rede, OneDrive?

### 2. Logs e Erros
- [ ] **Logs capturados**: Executar com logging habilitado
- [ ] **Mensagens de erro**: Texto exato do erro
- [ ] **Event Viewer**: Crashes registrados?
- [ ] **Console output**: Alguma mensagem visível?

### 3. Reprodução
- [ ] **Passos exatos**: Como reproduzir o problema?
- [ ] **Frequência**: Sempre ou intermitente?
- [ ] **Arquivos específicos**: Tipo, tamanho, localização?
- [ ] **Triggers**: Ação específica que causa?

## Problemas Comuns e Soluções

### 1. Aplicativo não inicia

#### Diagnóstico
```powershell
# Verificar dependências
Get-Item "libmpv-2.dll" -ErrorAction SilentlyContinue
Get-ItemProperty "HKLM:\SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}" -ErrorAction SilentlyContinue

# Verificar se executável existe
Test-Path ".\target\release\mtt-file-manager.exe"

# Tentar executar com logs
.\target\release\mtt-file-manager.exe 2>&1 | Select-String "error|Error|ERROR" | Select-Object -First 20
```

#### Soluções

**1.1. libmpv-2.dll não encontrada**
```powershell
# Download: https://sourceforge.net/projects/mpv-player-windows/files/libmpv/
# Copiar para mesmo diretório do executável
Copy-Item "libmpv-2.dll" -Destination ".\target\release\"

# Ou adicionar ao PATH do sistema
[Environment]::SetEnvironmentVariable("PATH", $env:PATH + ";C:\Path\To\MPV", "Machine")
```

**1.2. Runtime Visual C++**
```powershell
# Instalar VC++ Redistributable
winget install Microsoft.VCRedist.2015+.x64
winget install Microsoft.VCRedist.2015+.x86
```

**1.3. Cache corrompido**
```powershell
# Limpar cache e tentar novamente
Remove-Item "$env:LOCALAPPDATA\MTT-File-Manager" -Recurse -Force
```

### 2. Performance lenta / Travamentos

#### Diagnóstico
```powershell
# Verificar uso de recursos
Get-Process mtt-file-manager | Select-Object CPU, WorkingSet, VirtualMemorySize

# Verificar logs de performance
.\target\release\mtt-file-manager.exe 2>&1 | Select-String "frame_time|fps|PERF" | Tee-Object "perf.log"

# Verificar pasta atual
# Quantos arquivos?
(Get-ChildItem "C:\Path\To\Folder" -Recurse -ErrorAction SilentlyContinue).Count
```

#### Soluções

**2.1. Muitos arquivos na pasta**
- Sugerir navegar para subpastas menores
- Verificar se virtualização está funcionando (logs devem mostrar renderização parcial)
- Ajustar `thumbnail_size` para menor valor (64 ou 96)

**2.2. Thumbnails lentos**
```powershell
# Limpar cache de thumbnails
Remove-Item "$env:LOCALAPPDATA\MTT-File-Manager\thumbnails\*.webp"
```
- Reduzir tamanho de thumbnail padrão nas preferências
- Verificar se há arquivos corrompidos (logs de THUMB_STAGE*)

**2.3. Memory leak / Uso alto de memória**
- Verificar se há múltiplas instâncias rodando
- Reiniciar aplicativo
- Verificar logs por erros de cache
- Limpar cache se necessário

**2.4. Pasta na rede lenta**
- Usar view em lista em vez de grade (menos thumbnails)
- Desabilitar preview panel temporariamente
- Considerar mapear drive de rede como letra local

### 3. Thumbnails não aparecem

#### Diagnóstico
```powershell
# Verificar logs de thumbnail
.\target\release\mtt-file-manager.exe 2>&1 | Select-String "THUMB|ERROR" | Tee-Object "thumb_debug.log"

# Verificar codecs
Get-ItemProperty "HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\KindMap" | Select-String "video"

# Verificar se thumbnail específico existe no cache
$path = "C:\Path\To\File.jpg"
$bytes = [System.IO.File]::ReadAllBytes($path)
$hash = [System.BitConverter]::ToString($bytes).Replace("-", "").ToLower()
Test-Path "$env:LOCALAPPDATA\MTT-File-Manager\thumbnails\$hash.webp"
```

#### Soluções

**3.1. Formato não suportado**
- Verificar se arquivo abre em outros players/visualizadores
- Testar arquivo específico manualmente
- Verificar registry de codecs do Windows

**3.2. Arquivo corrompido**
- Testar com arquivo similar funcional
- Verificar integridade do arquivo
- Tentar abrir arquivo em outro programa

**3.3. Codec problem**
- Verificar Media Foundation está funcionando (testar no Movies & TV)
- Testar com arquivo de formato diferente
- Verificar se há atualizações do Windows

**3.4. Cache desatualizado**
```powershell
# Limpar cache de thumbnails
Remove-Item "$env:LOCALAPPDATA\MTT-File-Manager\thumbnails\*.webp"
# Ou remover tudo
Remove-Item "$env:LOCALAPPDATA\MTT-File-Manager" -Recurse -Force
```

### 4. Preview de vídeo não funciona

#### Diagnóstico
```powershell
# Verificar libmpv
Get-Item "libmpv-2.dll" | Select-Object VersionInfo, FullName

# Testar mpv diretamente
mpv.exe "caminho\para\video.mp4"

# Verificar logs de MPV
.\target\release\mtt-file-manager.exe 2>&1 | Select-String "MPV|video" | Tee-Object "mpv_debug.log"
```

#### Soluções

**4.1. libmpv não encontrada**
- Copiar DLL para diretório correto (mesmo do executável)
- Adicionar diretório da DLL ao PATH do sistema
- Verificar versão compatível (recomendada: latest stable)

**4.2. Formato de vídeo não suportado**
- Testar com arquivo MP4 padrão (H.264/AAC)
- Verificar se há codecs instalados no sistema
- Tentar converter arquivo para formato mais comum

**4.3. Erro de inicialização do player**
- Verificar se há múltiplas instâncias de vídeo abertas
- Reiniciar aplicação
- Verificar logs de MPV

### 5. PDF não abre

#### Diagnóstico
```powershell
# Verificar logs de PDF
.\target\release\mtt-file-manager.exe 2>&1 | Select-String "PDF" | Tee-Object "pdf_debug.log"

# Verificar versão do Windows (requer Windows 10+)
[System.Environment]::OSVersion.Version
```

#### Soluções

**5.1. Windows versão antiga**
- O visualizador de PDF usa a API nativa `Windows.Data.Pdf`, disponível a partir do Windows 10
- Atualizar o Windows se necessário

**5.2. Arquivo corrompido ou formato não suportado**
- Testar o arquivo em outro leitor de PDF
- Verificar se o arquivo não está protegido por senha (não suportado)
- Reiniciar após instalação

**5.2. PDF corrompido**
- Testar com PDF conhecido funcional
- Verificar se abre no navegador
- Verificar tamanho do arquivo (não deve ser 0 bytes)

**5.3. Permissões**
- Verificar se arquivo não está bloqueado (propriedades → desbloquear)
- Tentar copiar PDF para pasta local

### 6. Operações de arquivo falham

#### Diagnóstico
```powershell
# Verificar permissões
icacls "caminho\para\pasta"

# Verificar logs de operações
.\target\release\mtt-file-manager.exe 2>&1 | Select-String "FILE_OP|Access| denied" | Tee-Object "fileops.log"

# Verificar se arquivo está em uso
# (Requer Sysinternals Handle)
handle.exe "caminho\para\arquivo"
```

#### Soluções

**6.1. Permissões insuficientes**
- Executar como administrador (teste)
- Verificar ACLs da pasta/arquivo
- Verificar se arquivo não está em uso por outro programa

**6.2. Arquivo em uso**
- Fechar programas que podem estar usando o arquivo
- Usar Process Explorer para identificar
- Tentar operação após reiniciar

**6.3. Path muito longo**
- Mover arquivos para pasta com path mais curto
- Habilitar suporte a long paths no Windows (se aplicável)

### 7. Problemas de Navegação / Interface

#### Diagnóstico
```powershell
# Verificar logs de navegação
.\target\release\mtt-file-manager.exe 2>&1 | Select-String "NAV|input" | Tee-Object "nav.log"
```

#### Soluções

**7.1. Atalhos de teclado não funcionam**
- Verificar se outro programa não está capturando as teclas
- Tentar com teclado diferente
- Verificar se foco está na janela correta

**7.2. View não atualiza**
- Pressionar F5 para refresh manual
- Verificar se watcher está funcionando (logs de WATCHER)
- Navegar para outra pasta e voltar

**7.3. Aplicação congela**
- Verificar se está processando muitos arquivos
- Verificar uso de CPU/memória
- Forçar fechamento e reiniciar

## Perguntas Padrão para Tickets

### Informações do Sistema
1. Qual versão do Windows está usando? (execute `winver`)
2. Qual versão do MTT File Manager? (verificar `Cargo.toml` ou data do build)
3. Qual é a configuração do seu sistema? (CPU, RAM)
4. Os arquivos estão em disco local, rede ou OneDrive?

### Problema Específico
1. O que exatamente você estava tentando fazer?
2. O que você esperava que acontecesse?
3. O que realmente aconteceu?
4. O problema ocorre sempre ou apenas às vezes?

### Reprodução
1. Você pode reproduzir o problema consistentemente?
2. Quais são os passos exatos para reproduzir?
3. O problema ocorre com arquivos específicos ou qualquer arquivo?
4. O problema ocorre em pastas específicas?

### Logs e Erros
1. Você pode capturar os logs do aplicativo? (`run_with_logs.ps1`)
2. Há alguma mensagem de erro visível?
3. O aplicativo crashou? (verificar Event Viewer)
4. Você tem os arquivos de log gerados?

## Scripts de Diagnóstico

### Coleta Completa de Informações
```powershell
# collect_diagnostics.ps1
$timestamp = Get-Date -Format "yyyy-MM-dd_HH-mm-ss"
$folder = "MTT-Diag-$timestamp"
New-Item -ItemType Directory -Path $folder

# Info do sistema
systeminfo | Out-File "$folder\systeminfo.txt"
Get-ComputerInfo | Out-File "$folder\computerinfo.txt"
Get-WmiObject Win32_VideoController | Out-File "$folder\gpu.txt"

# Processo
Get-Process mtt-file-manager -ErrorAction SilentlyContinue | 
    Out-File "$folder\process.txt"

# Variáveis
@"
RUST_BACKTRACE=$env:RUST_BACKTRACE
LOCALAPPDATA=$env:LOCALAPPDATA
USERNAME=$env:USERNAME
"@ | Out-File "$folder\env.txt"

# Cache
$cache = "$env:LOCALAPPDATA\MTT-File-Manager"
if (Test-Path $cache) {
    Get-ChildItem $cache -Recurse | 
        Select-Object Name, Length, LastWriteTime |
        Out-File "$folder\cache.txt"
}

# Logs (se existirem)
Get-ChildItem "*.log" -ErrorAction SilentlyContinue | 
    Copy-Item -Destination $folder

Write-Host "Diagnostics collected in: $folder"
Compress-Archive -Path $folder -DestinationPath "$folder.zip"
Write-Host "Archive created: $folder.zip"
```

### Teste Rápido de Funcionalidades
```powershell
# quick_test.ps1
Write-Host "=== MTT File Manager Quick Test ==="

# Test 1: Executável existe
$exe = ".\target\release\mtt-file-manager.exe"
if (Test-Path $exe) {
    Write-Host "[PASS] Executable found" -ForegroundColor Green
} else {
    Write-Host "[FAIL] Executable not found at $exe" -ForegroundColor Red
}

# Test 2: libmpv-2.dll
if (Test-Path ".\target\release\libmpv-2.dll") {
    Write-Host "[PASS] libmpv-2.dll found" -ForegroundColor Green
} else {
    Write-Host "[FAIL] libmpv-2.dll not found" -ForegroundColor Red
}

# Test 3: WebView2
$webview2 = Get-ItemProperty "HKLM:\SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}" -ErrorAction SilentlyContinue
if ($webview2) {
    Write-Host "[PASS] WebView2 installed (version $($webview2.pv))" -ForegroundColor Green
} else {
    Write-Host "[WARN] WebView2 not detected" -ForegroundColor Yellow
}

# Test 4: Cache directory
$cache = "$env:LOCALAPPDATA\MTT-File-Manager"
if (Test-Path $cache) {
    Write-Host "[PASS] Cache directory exists" -ForegroundColor Green
} else {
    Write-Host "[INFO] Cache directory will be created on first run" -ForegroundColor Cyan
}

Write-Host "===================================="
```

## Procedimentos de Escalada

### Nível 1: Suporte Básico
- Coletar logs e informações do sistema
- Tentar soluções do playbook
- Verificar se é problema conhecido

### Nível 2: Suporte Técnico
- Analisar logs detalhados
- Reproduzir em ambiente de teste
- Verificar código fonte se necessário

### Nível 3: Desenvolvimento
- Issues no repositório
- Análise de código
- Fix e release

## Referências Rápidas

### Códigos de Erro Comuns
| Erro | Significado | Solução |
|------|-------------|---------|
| "libmpv-2.dll not found" | DLL não encontrada | Copiar DLL para pasta do exe |
| "WebView2 not available" | Runtime não instalado | Instalar WebView2 |
| "Access denied" | Sem permissões | Executar como admin/verificar ACLs |
| "File in use" | Arquivo bloqueado | Fechar outros programas |

### Atalhos de Debug
| Atalho | Função |
|--------|--------|
| F5 | Refresh manual |
| F2 | Renomear item selecionado |
| Delete | Mover para lixeira |
| Shift+Delete | Deletar permanentemente |
| Ctrl+R | Recarregar pasta |

---

### 9. Visualizador de Imagens Dedicado

#### Diagnóstico
```powershell
# Verificar se processo do viewer está rodando
Get-Process mtt-file-manager | Select-Object Id, CPU, WorkingSet

# Logs do viewer (stderr do processo separado)
.\target\release\mtt-file-manager.exe --image-viewer "C:\caminho\imagem.jpg" 2>&1 | Tee-Object "viewer.log"
```

#### Problemas e Soluções

**9.1. Spinner ao abrir imagem**
- **Causa**: Decodificação síncrona da primeira imagem falhou ou demorou
- **Solução**: Verificar se o arquivo de imagem é válido e acessível

**9.2. Spinner durante navegação rápida**
- **Causa**: Cache miss em navegação muito rápida (mais rápido que decodificação)
- **Comportamento esperado**: A imagem anterior deve permanecer visível até a nova estar pronta
- **Se aparecer spinner**: Verificar se `try_show_cached_current()` não está limpando textura

**9.3. Uso alto de memória no viewer**
- **Comportamento normal**: Até 512MB para cache de imagens (budget configurado)
- **Se exceder**: Verificar `MAX_CACHE_BYTES` e `evict_over_budget()` em `cache.rs`
- **Nota**: Memória é liberada pelo SO ao fechar o viewer (processo separado)

**9.4. Imagens não carregam (tela preta)**
- **Causa possível**: Formato não suportado, arquivo corrompido, EXIF malformado
- **Debug**: Verificar logs `[IMAGE-VIEWER]` para erros de decodificação
- **Fallback**: O loader tenta WIC como fallback se o image crate falhar

**9.5. Viewer não abre**
- **Causa**: Falha ao spawnar processo (`Command::new` falhou)
- **Debug**: Verificar se o executável existe no path esperado
- **Nota**: O viewer é o mesmo binário com flag `--image-viewer`

---

*Última atualização: 2026-02-24 (adicionada seção de troubleshooting do visualizador de imagens dedicado)*
