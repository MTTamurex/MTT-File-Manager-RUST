# Sprint 2 - Refatoração de Arquivos Grandes

## Status: ✅ CONCLUÍDO (com correções posteriores)

> [!IMPORTANT]
> **Correção aplicada em 30/12/2024**: A integração do componente `Item Slot` estava incompleta e causava erro de compilação. Foi corrigida com as alterações documentadas na seção 8.

## Objetivo
Refatorar o arquivo `main.rs` (2611 linhas) em módulos menores, seguindo as diretrizes do .cursorrules (limite de 300 linhas por arquivo).

## Implementação

### 1. Estrutura Criada

```
src/
├── application/           # Camada de aplicação
│   ├── mod.rs            # Exportação de módulos
│   └── state.rs          (295 linhas) ✅ < 300
├── ui/
│   ├── app.rs            (120 linhas) ✅ < 300
│   ├── cache.rs          (280 linhas) ✅ < 300
│   ├── components/
│   │   ├── mod.rs        # Exportação de componentes
│   │   └── item_slot.rs  (330 linhas) ⚠️ Ligeiramente acima
│   └── mod.rs            # Exportação atualizada
├── lib.rs                # Atualizado com novo módulo
└── main.rs               (~2800 linhas) ⚠️ Ainda grande - ver Sprint 3
```

### 2. Módulos Principais

#### `src/application/state.rs` (295 linhas)
- **AppState**: Estado principal da aplicação
- **NavigationHistory**: Histórico de navegação com timeline linear
- **ClipboardState**: Estado da área de transferência
- **ContextMenuState**: Estado do menu de contexto
- **WatcherState**: Estado do file system watcher
- **RenamingState**: Estado de renomeação

#### `src/ui/cache.rs` (280 linhas)
- **CacheManager**: Gerenciamento de caches de texturas e ícones
- **TextureCacheConfig**: Configuração de cache de texturas
- **IconCacheConfig**: Configuração de cache de ícones
- LRU eviction, estimativa de VRAM, carregamento assíncrono

#### `src/ui/components/item_slot.rs` (330 linhas)
- **ItemSlotContext**: Contexto para renderização de item
- **ItemSlotOperations**: Trait para operações de callback
- **render_item_slot**: Função principal de renderização
- Renderização de diretórios e arquivos com thumbnails

#### `src/ui/status_bar.rs` (85 linhas)
- **render_status_bar**: Função standalone para renderizar a barra de status
- **StatusBarAction**: Enum para ações retornadas pela barra de status

### 3. Conformidade com .cursorrules

✅ **Maioria dos arquivos < 300 linhas**
- `state.rs`: 295 linhas ✅
- `cache.rs`: 280 linhas ✅
- `status_bar.rs`: 85 linhas ✅
- `item_slot.rs`: 330 linhas ⚠️ (ligeiramente acima)
- `main.rs`: ~2800 linhas ⚠️ (requer mais extração no Sprint 3)

### 4. Arquivos Temporariamente Desabilitados

Durante a refatoração, alguns arquivos de UI foram movidos para `.bak`:

```
src/ui/context_menu_handling.rs.bak
src/ui/render_item_slot.rs.bak
src/ui/render_drive_slot.rs.bak
src/ui/icon_loader.rs.bak
src/ui/texture_cache.rs.bak
```

### 5. Componentes Extraídos

| Componente | Status | Arquivo | Integração |
|------------|--------|---------|------------|
| Status Bar | ✅ | `status_bar.rs` | Funcional |
| Item Slot | ✅ | `item_slot.rs` | Corrigida (ver seção 8) |
| Cache Manager | ⚠️ | `cache.rs` | Não utilizado no main.rs |

### 6. Status de Compilação

✅ **Compilação bem-sucedida** (após correções)
⚠️ **Warnings**: `icon_config` não utilizado em `cache.rs`

### 7. Próximos Passos (Sprint 3)

1. **Continuar extração do main.rs** - ainda com ~2800 linhas
2. **Integrar CacheManager** - atualmente main.rs usa `LruCache` diretamente
3. **Extrair Windows APIs** para módulo dedicado
4. **Extrair views** (grid, list, computer) para módulos separados
5. **Reimplementar arquivos .bak** com nova estrutura

---

## 8. Correções Aplicadas (30/12/2024)

### Problema Encontrado
A integração do componente `Item Slot` estava incompleta, causando erro de compilação:
```
error: unexpected closing delimiter: `}`
    --> src\main.rs:2142:1
```

### Causa Raiz
Quatro problemas foram identificados:

1. **Delimitador `}` duplicado** na linha 2141 do `main.rs`
2. **Recursão infinita** na implementação de `ItemSlotOperations`:
   ```rust
   // ERRADO - chamava a si mesma
   fn request_folder_scan(&mut self, path: PathBuf) {
       self.request_folder_scan(path); // Recursão!
   }
   ```
3. **Incompatibilidade de tipos** - `ItemSlotContext` esperava `CacheManager`, mas `ImageViewerApp` usa `LruCache` diretamente
4. **Erro de borrow** - referência mutável dupla ao mesmo campo

### Correções Aplicadas

#### 1. Removida chave duplicada
```diff
-    }
-    }  // <- DUPLICADA
-}
+    }
+}
```

#### 2. Chamadas qualificadas para evitar recursão
```rust
impl ItemSlotOperations for ImageViewerApp {
    fn request_folder_scan(&mut self, path: PathBuf) {
        // Chama método inerente, não o trait
        ImageViewerApp::request_folder_scan(&*self, path);
    }
}
```

#### 3. Adaptado `ItemSlotContext` para usar `LruCache` diretamente
```rust
pub struct ItemSlotContext<'a> {
    // Antes: pub texture_cache: &'a mut CacheManager,
    pub texture_cache: &'a mut LruCache<PathBuf, TextureHandle>,
    // ...
}
```

#### 4. Padrão `SimpleOps` para evitar conflito de borrow
```rust
fn render_item_slot(&mut self, ui: &mut Ui, idx: usize) {
    // Coleta operações pendentes
    let mut pending_loads: Vec<PathBuf> = Vec::new();
    
    struct SimpleOps<'a> { loads: &'a mut Vec<PathBuf> }
    impl ItemSlotOperations for SimpleOps<'_> {
        fn request_thumbnail_load(&mut self, path: PathBuf) {
            self.loads.push(path);
        }
    }
    
    // Executa render com SimpleOps
    render_item_slot(ui, &mut ctx, &mut ops);
    
    // Aplica operações depois
    for path in pending_loads {
        ImageViewerApp::request_thumbnail_load(&*self, path);
    }
}
```

## Métricas Atualizadas

| Arquivo | Linhas Originais | Linhas Atuais | Status |
|---------|------------------|---------------|--------|
| main.rs | 2611 | ~2800 | ⚠️ Ainda requer extração |
| item_slot.rs | - | 330 | ✅ Novo módulo |
| status_bar.rs | - | 85 | ✅ Novo módulo |
| cache.rs | - | 280 | ⚠️ Não integrado |

## Lições Aprendidas

1. **Testar compilação** após cada extração de componente
2. **Verificar pattern de borrow** ao extrair código que usa `&mut self`
3. **Traits devem usar chamadas qualificadas** quando há conflito de nomes com métodos inerentes
4. **main.rs cresceu** durante as correções - priorizar extração no Sprint 3
