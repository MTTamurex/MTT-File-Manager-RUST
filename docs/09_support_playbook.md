# Playbook de Suporte - MTT File Manager

## Objetivo do Documento
Este documento fornece um playbook para triagem e resolução de problemas do MTT File Manager, incluindo checklists, perguntas padrão e procedimentos de suporte.

## Checklist de Triagem Inicial

### 1. Informações Básicas
- [ ] **Versão do aplicativo**: Qual release ou commit?
- [ ] **Sistema operacional**: Windows 10/11? Build?
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
```

#### Soluções
1. **libmpv-2.dll não encontrada**
   ```powershell
   # Download: https://sourceforge.net/projects/mpv-player-windows/
   # Copiar para mesmo diretório do executável
   Copy-Item "libmpv-2.dll" -Destination ".\target\release\"
   ```

2. **WebView2 Runtime não instalado**
   ```powershell
   winget install Microsoft.EdgeWebView2Runtime
   ```

3. **Runtime Visual C++**
   ```powershell
   winget install Microsoft.VCRedist.2015+.{x64,x86}
   ```

### 2. Performance lenta / Travamentos

#### Diagnóstico
```powershell
# Verificar uso de recursos
Get-Process mtt-file-manager | Select-Object CPU, WorkingSet, VirtualMemorySize

# Verificar logs de performance
.\target\release\mtt-file-manager.exe 2>&1 | Select-String "frame_time|fps" | Tee-Object "perf.log"
```

#### Soluções
1. **Muitos arquivos na pasta**
   - Sugerir navegar para subpastas menores
   - Verificar se virtualização está funcionando
   - Ajustar `upload_budget_ms` para menor valor

2. **Thumbnails lentos**
   - Reduzir tamanho de thumbnail padrão
   - Desabilitar preview de vídeos grandes
   - Verificar se há arquivos corrompidos

3. **Memory leak**
   - Verificar se há múltiplas instâncias rodando
   - Reiniciar aplicativo
   - Verificar logs por erros de cache

### 3. Thumbnails não aparecem

#### Diagnóstico
```powershell
# Verificar logs de thumbnail
.\target\release\mtt-file-manager.exe 2>&1 | Select-String "THUMB|ERROR" | Tee-Object "thumb_debug.log"

# Verificar codecs
Get-ItemProperty "HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\KindMap" | Select-String "video"
```

#### Soluções
1. **Formato não suportado**
   - Verificar se arquivo abre em outros players
   - Testar arquivo específico manualmente
   - Verificar registry de codecs do Windows

2. **Arquivo corrompido**
   - Testar com arquivo similar funcional
   - Verificar integridade do arquivo
   - Tentar abrir arquivo em outro programa

3. **Codec problem**
   - Verificar Media Foundation está funcionando
   - Testar com arquivo de formato diferente
   - Verificar se há atualizações do Windows

### 4. Preview de vídeo não funciona

#### Diagnóstico
```powershell
# Verificar libmpv
Get-Item "libmpv-2.dll" | Select-Object VersionInfo

# Testar mpv diretamente
mpv.exe "caminho\para\video.mp4"
```

#### Soluções
1. **libmpv não encontrada**
   - Copiar DLL para diretório correto
   - Adicionar ao PATH do sistema
   - Verificar versão compatível

2. **Formato de vídeo não suportado**
   - Testar com arquivo MP4 padrão
   - Verificar se há codecs instalados
   - Tentar converter arquivo

### 5. PDF não abre

#### Diagnóstico
```powershell
# Verificar WebView2
Get-ItemProperty "HKLM:\SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}" | Select-Object pv

# Verificar logs
.\target\release\mtt-file-manager.exe 2>&1 | Select-String "PDF|WebView" | Tee-Object "pdf_debug.log"
```

#### Soluções
1. **WebView2 não instalado**
   - Instalar via winget ou manualmente
   - Verificar se Edge está atualizado
   - Reiniciar após instalação

2. **PDF corrompido**
   - Testar com PDF conhecido funcional
   - Verificar se abre no navegador
   - Verificar tamanho do arquivo

### 6. Operações de arquivo falham

#### Diagnóstico
```powershell
# Verificar permissões
icacls "caminho\para\pasta"

# Verificar logs de operações
.\target\release\mtt-file-manager.exe 2>&1 | Select-String "FILE_OP|Access" | Tee-Object "fileops.log"
```

#### Soluções
1. **Permissões insuficientes**
   - Executar como administrador (teste)
   - Verificar ACLs da pasta/arquivo
   - Verificar se arquivo não está em uso

2. **Arquivo em uso**
   - Fechar programas que podem estar usando
   - Verificar Process Explorer
   - Tentar operação após reiniciar

## Perguntas Padrão para Tickets

### Informações do Sistema
1. Qual versão do Windows está usando? (execute `winver`)
2. Qual versão do MTT File Manager? (release ou commit)
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
1. Você pode capturar os logs do aplicativo?
2. Há alguma mensagem de erro visível?
3. O aplicativo crashou? (Event Viewer)
4. Você tem os arquivos de log gerados?

### Contexto Adicional
1. O problema começou recentemente ou sempre existiu?
2. Algo mudou no seu sistema recentemente?
3. Você tem outros gerenciadores de arquivos instalados?
4. O problema ocorre em modo de segurança do Windows?

## Scripts de Diagnóstico

### Script de Coleta de Informações
```powershell
# diagnostic_info.ps1
Write-Host "=== MTT File Manager Diagnostic Info ===" -ForegroundColor Green

Write-Host "\n1. System Information:" -ForegroundColor Yellow
systeminfo | findstr /B /C:"OS Name" /C:"OS Version" /C:"System Type"

Write-Host "\n2. Hardware Information:" -ForegroundColor Yellow
Get-WmiObject win32_processor | Select-Object Name, NumberOfCores, MaxClockSpeed
Get-WmiObject win32_physicalmemory | Measure-Object -Property capacity -Sum | %{"RAM: $([math]::Round($_.sum/1GB,2)) GB"}

Write-Host "\n3. Dependencies Check:" -ForegroundColor Yellow
# WebView2
$webview2 = Get-ItemProperty "HKLM:\SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}" -ErrorAction SilentlyContinue
if ($webview2) { Write-Host "✓ WebView2 installed" -ForegroundColor Green }
else { Write-Host "✗ WebView2 not found" -ForegroundColor Red }

# libmpv (se no diretório atual)
if (Test-Path "libmpv-2.dll") { Write-Host "✓ libmpv-2.dll found" -ForegroundColor Green }
else { Write-Host "✗ libmpv-2.dll not found" -ForegroundColor Red }

Write-Host "\n4. Disk Space:" -ForegroundColor Yellow
Get-WmiObject win32_logicaldisk | Format-Table DeviceID, @{Name="Size(GB)";Expression={[math]::Round($_.Size/1GB,2)}}, @{Name="Free(GB)";Expression={[math]::Round($_.FreeSpace/1GB,2)}}

Write-Host "\n5. Cache Location:" -ForegroundColor Yellow
$cachePath = "$env:LOCALAPPDATA\MTT-File-Manager"
if (Test-Path $cachePath) {
    Write-Host "Cache exists: $cachePath"
    Get-ChildItem $cachePath -Recurse | Measure-Object -Property Length -Sum | %{"Total cache size: $([math]::Round($_.sum/1MB,2)) MB"}
} else {
    Write-Host "Cache not found (app may not have run yet)"
}
```

### Script de Teste de Performance
```powershell
# performance_test.ps1
Write-Host "=== MTT File Manager Performance Test ===" -ForegroundColor Green

# Criar pasta de teste
$testFolder = "$env:TEMP\mtt_test_$(Get-Random)"
New-Item -ItemType Directory -Path $testFolder | Out-Null

Write-Host "Creating test files..." -ForegroundColor Yellow
# Criar arquivos de teste
1..100 | ForEach-Object {
    $size = Get-Random -Minimum 1KB -Maximum 10MB
    $fileName = "test_$_.jpg"
    fsutil file createnew "$testFolder\$fileName" $size | Out-Null
}

Write-Host "Test folder created: $testFolder" -ForegroundColor Green
Write-Host "Navigate to this folder in MTT File Manager and monitor performance"
Write-Host "Press any key when done to cleanup..."
$null = $Host.UI.RawUI.ReadKey("NoEcho,IncludeKeyDown")

# Cleanup
Remove-Item $testFolder -Recurse -Force
Write-Host "Test files cleaned up" -ForegroundColor Green
```

## Procedimentos de Suporte

### Escalonamento de Issues

#### Nível 1 - Básico
- Verificar logs básicos
- Validar dependências
- Testar com arquivos/pastas diferentes
- Guiar usuário para coletar informações

#### Nível 2 - Intermediário
- Análise de logs detalhados
- Testes de performance
- Debugging de código específico
- Testes com builds de debug

#### Nível 3 - Avançado
- Análise de crash dumps
- Profiling de performance
- Debugging de threads/workers
- Modificação de código para testes

### Template de Resposta

```markdown
Obrigado pelo report! Vamos investigar este problema.

**Informações que precisamos:**
1. Logs do aplicativo (execute com: `.\target\release\mtt-file-manager.exe 2>&1 | Tee-Object "debug.log"`)
2. Versão exata do Windows (execute `winver`)
3. Tipo de arquivo/pasta que está causando o problema
4. Passos exatos para reproduzir

**Diagnóstico inicial:**
- [ ] Verificar se libmpv-2.dll está presente
- [ ] Verificar se WebView2 está instalado
- [ ] Testar com arquivos diferentes
- [ ] Verificar permissões de pasta

**Próximos passos:**
[Detalhar investigação específica baseada no problema]
```

## Recursos e Links

### Downloads
- **libmpv**: https://sourceforge.net/projects/mpv-player-windows/files/libmpv/
- **WebView2**: https://developer.microsoft.com/microsoft-edge/webview2/
- **Visual C++ Redistributable**: https://aka.ms/vs/17/release/vc_redist.x64.exe

### Ferramentas de Diagnóstico
- **Process Explorer**: https://docs.microsoft.com/sysinternals/downloads/process-explorer
- **DebugView**: https://docs.microsoft.com/sysinternals/downloads/debugview
- **WinDbg**: https://docs.microsoft.com/windows-hardware/drivers/debugger/debugger-download-tools

### Documentação
- **Windows APIs**: https://docs.microsoft.com/windows/win32/
- **egui documentation**: https://docs.rs/egui/
- **Rust debugging**: https://doc.rust-lang.org/book/ch09-00-error-handling.html

## Checklist Final de Suporte

### Antes de Fechar Ticket
- [ ] Problema foi resolvido?
- [ ] Usuário confirmou solução?
- [ ] Documentação foi atualizada se necessário?
- [ ] Issue foi adicionada a "known issues" se aplicável?
- [ ] Solução foi compartilhada com equipe?

### Métricas de Suporte
- [ ] Tempo de resposta inicial
- [ ] Tempo até resolução
- [ ] Complexidade do problema (1-5)
- [ ] Satisfação do usuário
- [ ] Lições aprendidas