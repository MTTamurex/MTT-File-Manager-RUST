# 🔒 Segurança no Windows - MTT File Manager

## Visão Geral

Este documento detalha todas as considerações de segurança para aplicações Desktop nativas no Windows, com foco em **prevenção de vulnerabilidades** comuns em gerenciadores de arquivos.

---

## ⚠️ Vetores de Ataque em File Managers

### 1. Path Traversal (Directory Traversal)

**Descrição**: Atacante tenta acessar arquivos fora do diretório permitido usando `..`, `/`, `\`.

**Exemplos de Input Malicioso**:
```
C:\Users\Public\Pictures\..\..\..\Windows\System32\config\SAM
C:/../../etc/passwd
\\?\C:\Windows\System32\important.dll
```

**Status Atual no Projeto**: ⚠️ **Parcialmente Mitigado**

**Implementação Atual**:
```rust
// Usa walkdir com max_depth(1) - previne recursão não autorizada
WalkDir::new(&path).max_depth(1)

// Mas não valida o path inicial!
self.current_path = path.to_string();  // ❌ SEM VALIDAÇÃO!
```

**✅ Solução Recomendada**:
```rust
use std::path::{Path, PathBuf};
use std::fs::canonicalize;

fn sanitize_path(input: &str) -> Result<PathBuf, SecurityError> {
    let path = Path::new(input);
    
    // 1. Canonicaliza (resolve .., symlinks, etc.)
    let canonical = canonicalize(path)
        .map_err(|_| SecurityError::InvalidPath)?;
    
    // 2. Verifica se está em disco válido (C:\, D:\, etc.)
    let drive = canonical
        .components()
        .next()
        .ok_or(SecurityError::InvalidPath)?;
    
    // 3. Bloqueia paths sensíveis
    let forbidden = [
        r"C:\Windows\System32",
        r"C:\Windows\SysWOW64",
        r"C:\Program Files\WindowsApps",
    ];
    
    for blocked in forbidden {
        if canonical.starts_with(blocked) {
            return Err(SecurityError::ForbiddenPath);
        }
    }
    
    Ok(canonical)
}
```

---

### 2. Command Injection via ShellExecuteW

**Descrição**: Executar arquivos maliciosos disfarçados de imagens/vídeos.

**Status Atual**: ⚠️ **Vulnerável**

**Código Atual**:
```rust
fn open_with_shell(path: &Path) {
    unsafe {
        let path_str = path.to_string_lossy().to_string();
        let path_wide: Vec<u16> = path_str
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        
        // ❌ PERIGO: Pode executar .bat, .cmd, .exe, .vbs, etc.
        let _ = ShellExecuteW(
            None,
            PCWSTR(std::ptr::null()),  // "open" implícito
            PCWSTR(path_wide.as_ptr()),
            // ...
        );
    }
}
```

**Vetores de Ataque**:
- `malware.jpg.exe` (dupla extensão)
- `image.png` (executável com ícone de imagem)
- `video.mp4.bat` (script batch)

**✅ Solução: Whitelist de Extensões**:
```rust
const SAFE_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "bmp", "webp",
    "mp4", "mkv", "avi", "mov", "webm",
];

fn open_with_shell(path: &Path) -> Result<(), SecurityError> {
    // 1. Valida extensão
    let ext = path.extension()
        .and_then(|e| e.to_str())
        .ok_or(SecurityError::InvalidExtension)?;
    
    if !SAFE_EXTENSIONS.contains(&ext.to_lowercase().as_str()) {
        return Err(SecurityError::UnsafeExtension);
    }
    
    // 2. Verifica se é realmente arquivo (não directory)
    if !path.is_file() {
        return Err(SecurityError::NotAFile);
    }
    
    // 3. Então executa
    unsafe {
        // ... ShellExecuteW ...
    }
    
    Ok(())
}
```

---

### 3. Symlink Attacks (Junction Points)

**Descrição**: Windows suporta **Symbolic Links** e **Junction Points** que podem apontar para qualquer lugar no filesystem.

**Cenário de Ataque**:
```powershell
# Atacante cria junction point malicioso
mklink /J "C:\Users\Public\Pictures\Fake Folder" "C:\Windows\System32"

# Quando usuário abre "Fake Folder", vê System32!
```

**Status Atual**: ❌ **Vulnerável**

**Código Atual**:
```rust
WalkDir::new(&path)
    // ❌ Segue symlinks por padrão!
```

**✅ Solução: Bloquear Symlinks**:
```rust
use std::fs;

WalkDir::new(&path)
    .follow_links(false)  // Não segue symlinks
    .into_iter()
    .filter_map(|e| e.ok())
    .filter(|e| {
        // Verificação adicional: rejeita reparse points
        if let Ok(metadata) = e.metadata() {
            // No Windows, symlinks têm FILE_ATTRIBUTE_REPARSE_POINT
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                let attrs = metadata.file_attributes();
                if (attrs & 0x400) != 0 {  // FILE_ATTRIBUTE_REPARSE_POINT
                    return false;
                }
            }
            true
        } else {
            false
        }
    })
```

---

### 4. Arquivos Hidden/System Maliciosos

**Descrição**: Malware pode se esconder em arquivos com atributos especiais.

**Status Atual**: ✅ **Mitigado**

**Implementação Atual**:
```rust
unsafe {
    use windows::Win32::Storage::FileSystem::{
        GetFileAttributesW,
        FILE_ATTRIBUTE_HIDDEN,
        FILE_ATTRIBUTE_SYSTEM,
        INVALID_FILE_ATTRIBUTES
    };
    
    let path_str = entry_path.to_string_lossy().to_string();
    let path_wide: Vec<u16> = path_str
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let attrs = GetFileAttributesW(PCWSTR(path_wide.as_ptr()));
    
    // ✅ Bloqueia arquivos hidden/system
    if attrs != INVALID_FILE_ATTRIBUTES {
        if (attrs & FILE_ATTRIBUTE_HIDDEN.0) != 0 || 
           (attrs & FILE_ATTRIBUTE_SYSTEM.0) != 0 {
            return false;
        }
    }
}

// ✅ Belt-and-suspenders: também filtra por nome
if name_str.starts_with('.') {
    return false;
}

// ✅ Bloqueia arquivos de sistema conhecidos
if matches!(name_str.to_lowercase().as_str(), 
    "desktop.ini" | "thumbs.db" | "$recycle.bin" | 
    "system volume information") {
    return false;
}
```

**Pontos de Melhoria**:
- Adicionar `FILE_ATTRIBUTE_TEMPORARY`
- Adicionar `FILE_ATTRIBUTE_OFFLINE`
- Adicionar `FILE_ATTRIBUTE_ENCRYPTED` (opcional)

---

### 5. Race Conditions (TOCTOU)

**TOCTOU**: Time-Of-Check to Time-Of-Use

**Descrição**: Arquivo pode ser modificado entre verificação e uso.

**Cenário**:
```
1. App verifica: is_file() → true
2. Atacante substitui por symlink
3. App executa: ShellExecuteW() → executa malware
```

**Status Atual**: ⚠️ **Não Mitigado**

**✅ Solução: Atomic Operations**:
```rust
// Abrir arquivo com handle e verificar metadados via handle
use std::fs::File;
use std::os::windows::fs::OpenOptionsExt;
use windows::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;

fn safe_open(path: &Path) -> Result<File, SecurityError> {
    let file = File::open(path)
        .map_err(|_| SecurityError::CannotOpen)?;
    
    let metadata = file.metadata()
        .map_err(|_| SecurityError::CannotReadMetadata)?;
    
    // Valida via handle (não via path)
    if !metadata.is_file() {
        return Err(SecurityError::NotAFile);
    }
    
    Ok(file)
}
```

---

### 6. Memória e Crash Safety

**Descrição**: Thumbnails corrompidos ou arquivos malformados podem crashar o app.

**Status Atual**: ⚠️ **Parcialmente Tratado**

**Código Atual**:
```rust
let (thumbnail_data, width, height) = extract_windows_thumbnail(&path)
    .unwrap_or_else(|_| create_error_placeholder());
    // ✅ Fallback em caso de erro
```

**Problemas Restantes**:
- HBITMAP inválido pode crashar `GetDIBits`
- Texturas gigantes (>4K) podem causar OOM
- COM initialization failure não tratado

**✅ Solução: Defense in Depth**:
```rust
fn extract_windows_thumbnail(path: &PathBuf) 
    -> Result<(Vec<u8>, u32, u32), ThumbnailError> {
    unsafe {
        // 1. Valida path length (Windows MAX_PATH = 260)
        if path.as_os_str().len() > 260 {
            return Err(ThumbnailError::PathTooLong);
        }
        
        // 2. Timeout para operações COM
        use std::time::Duration;
        let timeout = Duration::from_secs(5);
        
        // 3. Valida tamanho do HBITMAP
        let mut bm = BITMAP::default();
        GetObjectW(hbitmap, ...);
        
        if bm.bmWidth > 4096 || bm.bmHeight > 4096 {
            return Err(ThumbnailError::TextureTooLarge);
        }
        
        // 4. Valida buffer size antes de alocar
        let buffer_size = (bm.bmWidth * bm.bmHeight * 4) as usize;
        if buffer_size > 100 * 1024 * 1024 {  // 100 MB max
            return Err(ThumbnailError::OutOfMemory);
        }
        
        // ...
    }
}
```

---

## 🛡️ Permissões e Privilégios

### UAC (User Account Control)

**Status Atual**: ✅ Executa como **usuário normal** (não requer admin)

**Manifesto Recomendado** (`app.manifest`):
```xml
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <!-- Sempre roda como usuário normal -->
        <requestedExecutionLevel level="asInvoker" uiAccess="false"/>
      </requestedPrivileges>
    </security>
  </trustInfo>
</assembly>
```

### Sandbox (Futuro)

**Opção 1: AppContainer (Windows 10+)**
```rust
// Requer recompilação com flags especiais
// Limita acesso a filesystem, registry, network
```

**Opção 2: Low Integrity Level**
```rust
// Processo roda com menos privilégios que usuário
// Não pode escrever em locais protegidos
```

---

## 📋 Checklist de Segurança (Atualizado: 2026-01-01)

### ✅ Implementado

- [x] **Filtra arquivos hidden/system** via `GetFileAttributesW`
- [x] **Filtra extensões conhecidas** (whitelist) - implementado em `open_with_shell`
- [x] **Fallback para thumbnails corrompidos** - `create_error_placeholder()`
- [x] **LRU Cache previne OOM** - Capacidade limitada a 200 texturas
- [x] **Execução como usuário normal** (não requer admin)
- [x] **Depth limit no WalkDir** (`max_depth(1)`) - previne recursão não autorizada
- [x] **Exclusão segura via Lixeira** - `SHFileOperationW` com `FO_DELETE`
- [x] **Renomeação nativa** - `SHFileOperationW` com suporte a Undo (Ctrl+Z)
- [x] **Módulo de segurança** - `src/infrastructure/security.rs` criado (estrutura básica)

### 🚧 Parcialmente Implementado

- [~] **Sanitização de paths** - Implementação básica em `security.rs`, mas não integrada
- [~] **Validação de extensão** - Implementada em `open_with_shell`, mas não abrangente
- [~] **Bloqueio de symlinks** - `WalkDir::new(&path).follow_links(false)`, mas sem detecção de reparse points
- [~] **Tratamento de erros COM** - Fallbacks básicos, mas sem logging estruturado

### ❌ Faltando

- [ ] **Detecção de reparse points** via metadata (FILE_ATTRIBUTE_REPARSE_POINT)
- [ ] **Timeout em operações COM** - Sem mecanismo de timeout para operações bloqueantes
- [ ] **Validação de tamanho de texturas** - Sem limites para HBITMAPs gigantes (>4K)
- [ ] **Logging de tentativas de acesso suspeitas** - Sem sistema de auditoria
- [ ] **Rate limiting de operações de I/O** - Sem proteção contra DoS via I/O intensivo
- [ ] **Integração completa do módulo security** - Módulo criado mas não amplamente utilizado

### 📊 Estatísticas de Segurança

| Categoria | Status | Progresso |
|-----------|--------|-----------|
| **Path Sanitization** | 🟡 Parcial | 40% |
| **File Execution Safety** | 🟡 Parcial | 60% |
| **Symlink Protection** | 🟡 Parcial | 30% |
| **Memory Safety** | 🟢 Bom | 80% |
| **Error Handling** | 🟡 Parcial | 50% |
| **Audit & Logging** | 🔴 Fraco | 10% |

### 🎯 Prioridades Imediatas (Sprint de Segurança)

1. **Integrar `security.rs`** em todas as operações de filesystem
2. **Implementar detecção de reparse points** para bloquear symlinks
3. **Adicionar timeout** para operações COM (thumbnails, ícones)
4. **Implementar logging** com `tracing` para auditoria de segurança
5. **Validação de tamanho** para HBITMAPs e buffers de textura

---

## 🔍 Auditoria de Código Unsafe

**Total de blocos `unsafe` no projeto**: 12

### Justificativa de Cada Uso:

| Localização | Justificativa | Risco |
|------------|---------------|-------|
| `get_all_drives()` | FFI para `GetLogicalDriveStringsW` | Baixo - buffer fixo |
| `extract_windows_thumbnail()` | FFI para COM APIs | Médio - requer validação |
| `hbitmap_to_rgba()` | FFI para GDI | Médio - pointer manipulation |
| `hicon_to_rgba()` | FFI para GDI/GetIconInfo | Médio - converte HICON para RGBA |
| `extract_file_icon()` | FFI para SHGetFileInfoW | Baixo - usa dummy path + USEFILEATTRIBUTES |
| `extract_folder_icon_internal()` | FFI para SHGetFileInfoW | Baixo - usa dummy path |
| `extract_drive_icon()` | FFI para SHGetFileInfoW | Baixo - usa path real do drive |
| `get_volume_label()` | FFI para GetVolumeInformationW | Baixo - read-only |
| `open_with_shell()` | FFI para `ShellExecuteW` | Alto - execução de código |
| `load_folder()` (GetFileAttributesW) | FFI para filesystem APIs | Baixo - read-only |
| `rename_with_shell()` | FFI para `SHFileOperationW` | Médio - move/rename |
| `delete_with_shell()` | FFI para `SHFileOperationW` | Médio - move para lixeira |
| `CoInitializeEx/CoUninitialize` | COM initialization | Baixo - padrão documentado |

**Mitigações**:
- **SAFETY comments**: Todos os blocos `unsafe` possuem comentários obrigatórios documentando invariantes. ✅
- Todos os `unsafe` estão em funções pequenas e auditáveis
- Sem pointer arithmetic manual (usa `Vec<u8>`)
- Sem `transmute` ou type punning
- Todos os buffers têm bounds checking via Rust

---

## 🎯 Recomendações de Prioridade

### 🔥 Crítico (Implementar Imediatamente)

1. **Sanitização de paths** antes de navegar
2. **Validação de extensões** antes de `ShellExecuteW`
3. **Tratamento de erros COM** com logging

### ⚠️ Importante (Próxima Release)

4. Bloqueio de symlinks/reparse points
5. Timeout em operações de I/O
6. Validação de tamanho de texturas

### 📋 Desejável (Futuro)

7. AppContainer sandbox
8. Code signing do executável
9. Auto-update seguro

---

## 📚 Referências

- [Microsoft Security Development Lifecycle](https://www.microsoft.com/en-us/securityengineering/sdl)
- [OWASP Desktop Security](https://owasp.org/www-community/vulnerabilities/Path_Traversal)
- [Rust Secure Coding Guidelines](https://anssi-fr.github.io/rust-guide/)
- [Windows Security Best Practices](https://docs.microsoft.com/en-us/windows/security/)

---

## ⚙️ Como Reportar Vulnerabilidades

Se você encontrar uma vulnerabilidade de segurança, **NÃO** abra uma issue pública. Entre em contato diretamente com os maintainers.
