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

## 📅 Histórico de Atualizações

| Data | Padrão | Descrição |
|------|--------|-----------|
| 2025-12-28 | Shrink-Wrap Selection | Solução completa para seleção que abraça conteúdo em grids |

---

*Última atualização: 2025-12-28*
*Responsável: MTT File Manager Team*
