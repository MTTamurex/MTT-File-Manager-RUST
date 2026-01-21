# Build, Execução e Deploy - MTT File Manager

## 1. Pré-requisitos de Desenvolvimento

### Toolchain Rust

```powershell
# Instalar Rust (via rustup)
winget install Rustlang.Rustup

# Verificar instalação
rustc --version
cargo --version
```

**Requisitos:**
- Rust Edition 2021
- Target: `x86_64-pc-windows-msvc`

### Ferramentas de Build

| Ferramenta | Propósito | Instalação |
|------------|-----------|------------|
| MSVC Build Tools | Compilação C/C++ | Visual Studio Installer |
| Windows SDK | Headers do Windows | Visual Studio Installer |
| cargo | Gerenciador de pacotes Rust | Via rustup |

### Dependências Externas

1. **libmpv**
   - `mpv.lib` já está presente na raiz do projeto
   - Runtime: `libmpv-2.dll` necessário para execução

2. **SQLite**
   - Bundled via feature `rusqlite/bundled`
   - Não requer instalação separada

---

## 2. Compilação

### Build de Desenvolvimento

```powershell
# Na raiz do projeto
cd c:\Users\mtamu\github\MTT-File-Manager-RUST

# Compilar em modo debug
cargo build

# Resultado: target\debug\mtt-file-manager.exe
```

**Características do Debug Build:**
- Sem otimizações
- Símbolos de debug incluídos
- Build rápido (~30s incremental)

### Build de Release

```powershell
# Compilar em modo release
cargo build --release

# Resultado: target\release\mtt-file-manager.exe
```

**Características do Release Build:**
- Otimização máxima (`opt-level=3`)
- Link-Time Optimization (`lto=true`)
- Single codegen unit
- Build lento (~2-5 min primeira vez)
- Binário menor e muito mais rápido

### Verificar Dependências

```powershell
# Verificar se compila sem erros
cargo check

# Verificar warnings e clippy
cargo clippy
```

---

## 3. Execução

### Execução Direta

```powershell
# Modo debug
cargo run

# Modo release
cargo run --release

# Ou executar binário diretamente
.\target\release\mtt-file-manager.exe
```

### Execução com Logs

Use o script incluído:

```powershell
.\run_with_logs.ps1
```

**O que faz:**
1. Executa o binário release
2. Captura stdout e stderr
3. Salva em `debug_metadata.log`

### Teste de Portabilidade

Use o script incluído:

```powershell
.\test_standalone.ps1
```

**O que faz:**
1. Copia executável para pasta temporária
2. Verifica que não há dependência de `assets/`
3. Executa o programa isolado

---

## 4. Entradas Obrigatórias

### Arquivos de Sistema Requeridos

| Arquivo | Localização | Fallback |
|---------|-------------|----------|
| `segoeui.ttf` | `C:\Windows\Fonts\` | Fonte padrão do egui |
| `seguisym.ttf` | `C:\Windows\Fonts\` | Símbolos não renderizados |
| `libmpv-2.dll` | PATH ou junto ao .exe | Vídeo não funciona |

### Recursos Embarcados (Já incluídos)

- `remixicon.ttf` - Fonte de ícones
- `appicon.png` - Ícone da aplicação
- 28 ícones SVG - Toolbar e UI

---

## 5. Saídas e Artefatos

### Artefato Principal

```
target\release\mtt-file-manager.exe
```

**Tamanho estimado:** ~15-20MB (com LTO)

**Contém:**
- Executável standalone
- Ícone Windows embutido
- Assets embarcados (fontes, SVGs)
- SQLite bundled

### Artefatos de Runtime

| Artefato | Localização | Criado quando |
|----------|-------------|---------------|
| `thumbnail_cache.db` | `%APPDATA%\mtt-file-manager\` | Primeira execução |
| Logs (se habilitados) | Diretório atual | Com scripts de debug |

---

## 6. Scripts de Build

### `build.rs`

**Propósito:** Embed de recursos Windows

**Ações:**
1. Localiza `appicon.ico`
2. Cria recurso Windows com metadados
3. Compila recurso no executável

**Resultado:**
- Ícone visível no Explorer
- Propriedades do arquivo com metadados

---

## 7. Distribuição

### Distribuição Simples

Para distribuir o aplicativo:

1. **Copiar arquivos:**
   ```
   mtt-file-manager.exe
   libmpv-2.dll  (se não estiver no PATH)
   ```

2. **Opcionalmente incluir:**
   - Codecs de vídeo (K-Lite, etc.)

### Verificação de Dependências

```powershell
# Verificar DLLs necessárias (requer Visual Studio)
dumpbin /dependents target\release\mtt-file-manager.exe
```

**DLLs típicas requeridas:**
- `KERNEL32.dll`
- `USER32.dll`
- `GDI32.dll`
- `ole32.dll`
- `SHELL32.dll`
- `libmpv-2.dll` (se usado vídeo)

---

## 8. Problemas Conhecidos de Build

### Erro: "mpv.lib not found"

**Solução:** Verificar que `mpv.lib` está na raiz do projeto

### Erro: "link.exe not found"

**Solução:** Instalar MSVC Build Tools via Visual Studio Installer

### Erro: "cannot find Windows SDK"

**Solução:** Instalar Windows 10/11 SDK via Visual Studio Installer

### Warning: "unused import"

**Status:** Warnings de linting são esperados, não falham o build

---

## 9. Comandos Úteis

```powershell
# Build limpo (remove artefatos anteriores)
cargo clean
cargo build --release

# Verificar tamanho do binário
(Get-Item target\release\mtt-file-manager.exe).Length / 1MB

# Executar testes (se existirem)
cargo test

# Verificar dependências de crates
cargo tree

# Atualizar dependências
cargo update
```

---

## 10. Itens Não Documentados no Código

⚠️ **Os seguintes itens não possuem documentação explícita:**

1. **Versão mínima do Windows** - Não especificada
2. **Requisitos de memória** - Não documentados
3. **Instalador/Uninstaller** - Não existe
4. **Auto-update** - Não implementado
5. **Assinatura de código** - Não configurada
