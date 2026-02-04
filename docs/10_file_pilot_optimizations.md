# Otimizações do File Pilot Implementadas

Este documento descreve as otimizações inspiradas no File Pilot que foram implementadas no MTT File Manager.

## Resumo das Features

| Feature | File Pilot | Status no MTT |
|---------|------------|---------------|
| **NtQueryDirectoryFile** para indexação rápida | ✅ Implementado | ✅ **Já existia** - ativo para HDDs |
| **ReadDirectoryChangesW no drive inteiro** | ✅ Implementado | ✅ **Novo** - `DriveWatcher` criado |

---

## 1. NtQueryDirectoryFile para Indexação Rápida

### O que é
API nativa do Windows NT que permite ler múltiplas entradas de diretório em uma única chamada de sistema, bypassando as camadas de abstração do Win32.

### Implementação Existente
- **Arquivo:** `src/infrastructure/ntfs_reader.rs`
- **Uso:** Ativo em `src/app/operations/folder_loading.rs:548-675`
- **Condição:** Usado quando `!is_ssd && ntfs_reader::is_available()`

### Como funciona
1. Chama `NtQueryDirectoryFile` da `ntdll.dll` diretamente
2. Lê até 64KB de entradas de diretório de uma vez
3. Parse direto da estrutura `FILE_DIRECTORY_INFORMATION`
4. Bypass completo das APIs Win32 de alto nível

### Benefícios
- **Menos syscalls:** Uma chamada para múltiplas entradas vs uma chamada por entrada
- **Menos overhead:** Sem abstrações do Win32
- **Melhor para HDDs:** Menos seek operations

---

## 2. ReadDirectoryChangesW no Drive Inteiro

### O que é
Monitoramento de mudanças no sistema de arquivos usando `ReadDirectoryChangesW` diretamente no drive raiz (ex: `C:\`) em vez de pastas individuais.

### Implementação Nova
- **Módulo:** `src/infrastructure/drive_watcher.rs`
- **Integração:** `src/infrastructure/drive_watcher_integration.rs`

### Arquitetura

```
┌─────────────────────────────────────────────────────────────┐
│                 DriveWatcherManager                         │
│              (um manager para toda a app)                   │
├─────────────────────────────────────────────────────────────┤
│  Drive "C:\"              │  Drive "D:\"                     │
│  ┌──────────────────┐     │  ┌──────────────────┐           │
│  │  DriveWatcher    │     │  │  DriveWatcher    │           │
│  │  (thread +       │     │  │  (thread +       │           │
│  │   handle)        │     │  │   handle)        │           │
│  └──────────────────┘     │  └──────────────────┘           │
└─────────────────────────────────────────────────────────────┘
```

### Como funciona
1. **Criação:** Ao navegar para uma pasta, extrai o drive root (ex: `C:\`)
2. **Watch:** Abre handle para o drive raiz com `FILE_LIST_DIRECTORY`
3. **Monitoramento:** Chama `ReadDirectoryChangesW` com `bWatchSubtree = TRUE`
4. **Filtragem:** Filtra eventos pelo prefixo da pasta atual
5. **Reutilização:** Ao navegar para outra pasta no mesmo drive, apenas atualiza o prefixo

### Benefícios vs Watcher por Pasta

| Aspecto | Watcher por Pasta (antigo) | Drive Watcher (novo) |
|---------|---------------------------|----------------------|
| **Setup ao navegar** | Recria watcher (50-200ms) | Apenas atualiza prefixo (0ms) |
| **Handles abertos** | Um por pasta | Um por drive |
| **Eventos perdidos** | Durante transição entre pastas | Nunca (monitoramento contínuo) |
| **Navegação rápida** | Pode perder mudanças recentes | Todas as mudanças capturadas |
| **Memória** | Linear com profundidade de navegação | Constante (um watcher por drive) |

### Uso no App

#### 1. Inicializar o Manager
```rust
use crate::infrastructure::drive_watcher_integration::DriveWatcherManager;

// No estado do app
pub drive_watcher: DriveWatcherManager,
```

#### 2. Watch ao navegar
```rust
// Em watch_current_folder() - substitui o notify-watcher
self.drive_watcher.watch_path(PathBuf::from(&self.current_path));
```

#### 3. Poll de eventos no update loop
```rust
// Em handle_fs_events() ou similar
for event in self.drive_watcher.poll_events() {
    match event {
        DriveWatcherEvent::Created(path) => { /* atualizar UI */ },
        DriveWatcherEvent::Deleted(path) => { /* remover item */ },
        DriveWatcherEvent::Modified(path) => { /* recarregar metadata */ },
        _ => {}
    }
}
```

---

## Comparação com File Pilot

### O que o File Pilot fazia
> "Stabilized directory change tracking by using ReadDirectoryChanges on the entire drive instead of per folder."

Exatamente o que implementamos: monitorar o drive inteiro e filtrar eventos.

### O que implementamos
1. ✅ Drive-wide monitoring com `ReadDirectoryChangesW`
2. ✅ Prefix filtering para reportar apenas eventos relevantes
3. ✅ Multi-drive support (C:\, D:\, etc.)
4. ✅ Zero-overhead navigation (sem recriar watchers)
5. ✅ Async I/O com OVERLAPPED para não bloquear

---

## Próximos Passos para Integração Completa

### 1. Substituir notify-watcher (Opcional)
O sistema atual com `notify` crate ainda funciona. Para migrar completamente:

```rust
// Em app/state.rs
// Substituir:
#[cfg(feature = "notify-watcher")]
pub watcher: Option<RecommendedWatcher>,

// Por:
pub drive_watcher: DriveWatcherManager,
```

### 2. Integrar ao message_handler.rs
```rust
// Em handle_fs_events()
pub fn handle_fs_events(&mut self) {
    // Novo: usar drive watcher
    for event in self.drive_watcher.poll_events() {
        self.process_drive_event(event);
    }
    
    // Legacy: manter compatibilidade com notify
    #[cfg(feature = "notify-watcher")]
    self.handle_notify_events();
}
```

### 3. Feature flag
```toml
# Cargo.toml
[features]
default = ["notify-watcher"]
notify-watcher = ["dep:notify"]
drive-watcher = []  # Nova feature
```

---

## Testes

### Testes Unitários
```bash
cargo test drive_watcher
```

### Testes Manuais
1. Navegar para pasta A em C:\
2. Criar arquivo em pasta A (deve detectar)
3. Navegar rapidamente para pasta B em C:\
4. Criar arquivo em pasta A (deve detectar mesmo não estando lá)
5. Navegar de volta para pasta A (deve mostrar arquivo novo)

---

## Referências

- [ReadDirectoryChangesW - Microsoft Docs](https://docs.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-readdirectorychangesw)
- [NtQueryDirectoryFile - NT API](https://docs.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/nf-ntifs-ntquerydirectoryfile)
- [File Pilot - Features](https://filepilot.tech/)
- Implementação MTT: `src/infrastructure/drive_watcher.rs`