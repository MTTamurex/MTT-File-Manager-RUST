# 🔧 Padrões Reutilizáveis - MTT File Manager

Este documento contém soluções de problemas comuns que podem ser aplicadas em outros projetos.

---

## 📦 Sumário

1. [Shrink-Wrap Selection em Grid UI](#1-shrink-wrap-selection-em-grid-ui)
2. [Async Thumbnail Loading com LRU Cache](#2-async-thumbnail-loading-com-lru-cache)
3. [Virtualização de Lista com show_rows](#3-virtualização-de-lista-com-show_rows)

---

## 1. Shrink-Wrap Selection em Grid UI

### Problema
Em grids com altura de célula fixa, a seleção visual (highlight) ocupa toda a célula, incluindo espaço vazio abaixo do conteúdo. Isso cria uma aparência "desconexa" onde a seleção não corresponde ao conteúdo real.

### Solução: Bounding Box Decoupling

**Princípio:** Separar o retângulo estrutural (célula do grid) do retângulo visual (seleção).

```
┌─────────────────┐  ◄── cell_rect (altura fixa)
│  ┌───────────┐  │
│  │  Conteúdo │  │  ◄── selection_rect (altura dinâmica)
│  └───────────┘  │
│     ░░░░░░░░    │  ◄── Espaço não selecionado
└─────────────────┘
```

### Implementação Genérica

```rust
// 1. Aloca célula do grid (estrutural)
let (_, cell_rect) = ui.allocate_space(egui::vec2(cell_width, cell_height));

// 2. Calcula altura REAL do conteúdo
let content_height = calculate_content_height(item);

// 3. Cria selection_rect baseado no conteúdo
let selection_rect = egui::Rect::from_min_size(
    cell_rect.min,
    egui::vec2(cell_rect.width(), content_height.min(cell_rect.height()))
);

// 4. Pinta seleção no rect de conteúdo, NÃO na célula
if is_selected {
    ui.painter().rect_filled(selection_rect, rounding, selection_color);
}

// 5. Renderiza conteúdo dentro do selection_rect
ui.put(selection_rect, |ui| render_item(ui, item));
```

### Cálculo de Altura por Tipo de Conteúdo

O segredo está em **calcular a altura exata** do que será renderizado:

```rust
fn calculate_content_height(&self, item: &Item) -> f32 {
    // Mede altura do texto
    let text_height = measure_text_height(ui, &item.name, max_width, max_rows);
    
    match item.item_type {
        ItemType::Folder => {
            let icon_size = base_size * 0.6;  // Ícones de pasta são menores
            icon_size + gap + text_height + padding
        }
        ItemType::Image => {
            // Para imagens, considera aspect ratio!
            let img_height = if let Some(tex) = get_texture(&item.path) {
                let aspect = tex.width() / tex.height();
                if aspect > 1.0 {
                    base_size / aspect  // Paisagem: altura menor
                } else {
                    base_size  // Retrato: altura máxima
                }
            } else {
                base_size  // Placeholder/spinner
            };
            img_height + gap + text_height + padding
        }
        ItemType::GenericFile => {
            // Ícone genérico é 50% e centralizado com espaço acima
            let icon_size = base_size * 0.5;
            let top_space = (base_size - icon_size) / 2.0;
            top_space + icon_size + gap + text_height + padding
        }
    }
}
```

### Medição de Texto com TextWrapping

```rust
fn measure_text_height(ui: &Ui, text: &str, max_width: f32, max_rows: usize) -> f32 {
    let galley = ui.fonts(|fonts| {
        fonts.layout_job(egui::text::LayoutJob {
            text: text.to_string(),
            sections: vec![egui::text::LayoutSection {
                format: egui::TextFormat {
                    font_id: egui::FontId::proportional(10.0),
                    ..Default::default()
                },
                ..Default::default()
            }],
            wrap: egui::text::TextWrapping {
                max_width,
                max_rows,
                break_anywhere: true,
                ..Default::default()
            },
            ..Default::default()
        })
    });
    galley.rect.height().max(MIN_TEXT_HEIGHT)  // Garante altura mínima
}
```

### Troubleshooting

| Problema | Causa | Solução |
|----------|-------|---------|
| Seleção muito grande | `content_h` não corresponde ao render | Auditar render_item para usar mesmos valores |
| Seleção corta conteúdo | Faltou padding/margins | Adicionar gaps entre elementos |
| Imagens paisagem com espaço | Aspect ratio não detectado | Verificar se cache contém textura |
| Inconsistência visual | Diferentes fórmulas por tipo | Unificar lógica de padding |

### Quando Usar

- ✅ Grids com itens de tamanhos variados
- ✅ Listas com ícones + texto
- ✅ Qualquer UI onde seleção deve "abraçar" conteúdo
- ❌ Listas simples de altura uniforme
- ❌ Tabelas com colunas fixas

---

## 2. Async Thumbnail Loading com LRU Cache

### Problema
Carregar thumbnails de forma síncrona congela a UI. Carregar todos assincronamente pode sobrecarregar memória.

### Solução: Channel + LRU Cache + Visibility Check

```rust
// Estado
struct App {
    texture_cache: LruCache<PathBuf, TextureHandle>,
    loading_set: HashSet<PathBuf>,
    receiver: Receiver<(PathBuf, RgbaImage)>,
    sender: Sender<(PathBuf, RgbaImage)>,
}

// Na renderização
if is_visible && !cache.contains(&path) && !loading_set.contains(&path) {
    loading_set.insert(path.clone());
    spawn_thumbnail_load(path, sender.clone());
}

// No update loop
while let Ok((path, image)) = receiver.try_recv() {
    let texture = upload_to_gpu(ctx, &image);
    cache.put(path.clone(), texture);
    loading_set.remove(&path);
}
```

---

## 3. Virtualização de Lista com show_rows

### Problema
Renderizar milhares de itens é lento. Virtualização manual é complexa.

### Solução: egui::ScrollArea::show_rows

```rust
let row_height = 30.0;
let total_rows = items.len();

ScrollArea::vertical().show_rows(ui, row_height, total_rows, |ui, visible_range| {
    for row_idx in visible_range {
        // Só renderiza linhas visíveis
        render_row(ui, &items[row_idx]);
    }
});
```

### Para Grids (2D)

```rust
let items_per_row = (available_width / cell_width).floor() as usize;
let total_rows = (items.len() + items_per_row - 1) / items_per_row;

ScrollArea::vertical().show_rows(ui, row_height, total_rows, |ui, row_range| {
    for row in row_range {
        ui.horizontal(|ui| {
            let start = row * items_per_row;
            let end = (start + items_per_row).min(items.len());
            for i in start..end {
                render_cell(ui, &items[i]);
            }
        });
    }
});
```

---

## 4. Pattern de "Coleta de Ações" para Resolver Borrow Conflicts

### Problema
Em UI frameworks como egui, você frequentemente encontra conflitos de borrow quando precisa:
1. Modificar estado da aplicação durante renderização
2. Processar eventos de UI que requerem mutabilidade
3. Manter referências imutáveis para renderização

**Exemplo de conflito**:
```rust
// ❌ Isso não compila: cannot borrow `self` as mutable because it is also borrowed as immutable
fn render_ui(&mut self, ui: &mut egui::Ui) {
    let items = &self.items;  // Borrow imutável
    
    for item in items {
        if ui.button(&item.name).clicked() {
            self.delete_item(item);  // ❌ Tentativa de borrow mutável!
        }
    }
}
```

### Solução: Pattern de "Coleta de Ações"
Coletar ações em um `Vec<Action>` durante renderização e executá-las depois.

```rust
#[derive(Debug)]
enum Action {
    DeleteItem(PathBuf),
    RenameItem(PathBuf, String),
    NavigateTo(PathBuf),
}

struct App {
    items: Vec<FileEntry>,
    pending_actions: Vec<Action>,
}

impl App {
    fn render_ui(&mut self, ui: &mut egui::Ui) {
        // Limpa ações do frame anterior
        self.pending_actions.clear();
        
        // Coleta ações durante renderização
        let mut actions = Vec::new();
        
        for item in &self.items {
            ui.horizontal(|ui| {
                if ui.button("🗑️").clicked() {
                    actions.push(Action::DeleteItem(item.path.clone()));
                }
                
                if ui.button("✏️").clicked() {
                    actions.push(Action::RenameItem(item.path.clone(), item.name.clone()));
                }
            });
        }
        
        // Armazena ações para processamento posterior
        self.pending_actions = actions;
    }
    
    fn update(&mut self) {
        // Processa ações coletadas
        for action in self.pending_actions.drain(..) {
            match action {
                Action::DeleteItem(path) => self.delete_item(&path),
                Action::RenameItem(path, new_name) => self.rename_item(&path, &new_name),
                Action::NavigateTo(path) => self.navigate_to(&path),
            }
        }
    }
}
```

### Variação: Usando Closure Captures
```rust
fn render_view(&self, ui: &mut egui::Ui) -> Vec<Action> {
    let mut actions = Vec::new();
    
    // Closure captura actions por referência mutável
    let mut add_action = |action| actions.push(action);
    
    for item in &self.items {
        if ui.button("Delete").clicked() {
            add_action(Action::DeleteItem(item.path.clone()));
        }
    }
    
    actions
}

// No loop principal:
let actions = app.render_view(ui);
for action in actions {
    app.process_action(action);
}
```

### Benefícios
- ✅ Resolve conflitos de borrow em tempo de compilação
- ✅ Separa lógica de renderização de lógica de negócio
- ✅ Fácil de testar: ações são valores puros
- ✅ Suporta operações batch (processa múltiplas ações de uma vez)

### Quando Usar
- ✅ UI complexa com múltiplas interações
- ✅ Quando `RefCell` ou `Rc` seriam overkill
- ✅ Sistemas de eventos/commands
- ❌ UI simples com poucas interações

---

## 5. Icon Loader Assíncrono com Cache Unificado

### Problema
Carregar ícones do Windows (SHGetFileInfoW) é uma operação síncrona que pode travar a UI. Cada componente (sidebar, grid, list) tinha seu próprio cache, causando duplicação.

### Solução: Worker Thread + Cache Centralizado

```rust
// Estrutura do IconLoader
struct IconLoader {
    request_sender: Sender<IconRequest>,
    result_receiver: Receiver<IconResult>,
    cache: Arc<Mutex<LruCache<CacheKey, TextureHandle>>>,
}

impl IconLoader {
    fn new(ctx: egui::Context) -> Self {
        let (req_tx, req_rx) = mpsc::channel();
        let (res_tx, res_rx) = mpsc::channel();
        let cache = Arc::new(Mutex::new(LruCache::new(100)));
        
        // Worker thread
        std::thread::spawn(move || {
            while let Ok(request) = req_rx.recv() {
                let result = match request.icon_type {
                    IconType::Computer => extract_computer_icon(request.size),
                    IconType::Drive(path) => extract_drive_icon(&path, request.size),
                    IconType::File(path) => extract_file_icon(&path, request.size),
                    IconType::Folder(path) => extract_folder_icon(&path, request.size),
                };
                
                let _ = res_tx.send(IconResult {
                    request_id: request.request_id,
                    result,
                });
            }
        });
        
        Self {
            request_sender: req_tx,
            result_receiver: res_rx,
            cache,
        }
    }
    
    fn request_icon(&self, request: IconRequest) {
        let _ = self.request_sender.send(request);
    }
    
    fn process_results(&mut self, ctx: &egui::Context) {
        while let Ok(result) = self.result_receiver.try_recv() {
            if let Ok((pixels, width, height)) = result.result {
                let texture = ctx.load_texture(
                    format!("icon_{}", result.request_id),
                    ColorImage::from_rgba_unmultiplied([width as usize, height as usize], &pixels),
                    Default::default()
                );
                
                let mut cache = self.cache.lock().unwrap();
                cache.put(result.request_id, texture);
            }
        }
    }
    
    fn get_texture(&self, key: &CacheKey) -> Option<TextureHandle> {
        let cache = self.cache.lock().unwrap();
        cache.get(key).cloned()
    }
}
```

### Integração com UI
```rust
// Componentes usam o mesmo IconLoader
struct Sidebar {
    icon_loader: Arc<IconLoader>,
}

struct GridView {
    icon_loader: Arc<IconLoader>,  // Mesma instância compartilhada
}

// Inicialização
let icon_loader = Arc::new(IconLoader::new(ctx.clone()));
let sidebar = Sidebar::new(Arc::clone(&icon_loader));
let grid_view = GridView::new(Arc::clone(&icon_loader));
```

### Benefícios
- ✅ Cache unificado evita duplicação de texturas
- ✅ Worker thread previne travamento da UI
- ✅ Reutilização entre componentes
- ✅ LRU cache gerencia memória automaticamente

---

## 📅 Histórico de Atualizações

| Data | Padrão | Descrição |
|------|--------|-----------|
| 2025-12-28 | Shrink-Wrap Selection | Solução completa para seleção que abraça conteúdo em grids |
| 2026-01-01 | Coleta de Ações | Pattern para resolver conflitos de borrow em UI |
| 2026-01-01 | Icon Loader Assíncrono | Cache unificado com worker thread para ícones |

---

*Última atualização: 2026-01-01*
*Responsável: MTT File Manager Team*
*Versão: 2.0 - Adicionados novos padrões de arquitetura*
