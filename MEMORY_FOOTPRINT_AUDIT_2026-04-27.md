# Auditoria de Memory Footprint - MTT File Manager Rust - 2026-04-27

## Escopo

Auditoria estática feita somente sobre código-fonte real do projeto. Não usei documentação do projeto como base para os achados. O foco foi reduzir working set/RSS, alocações transitórias e memória de GPU/egui sem degradar CPU, responsividade, fluidez visual ou comportamento existente.

Não executei profiling dinâmico nesta etapa. Onde o impacto depende de carga real, a recomendação inclui pontos de medição antes da implementação.

## Resumo executivo

Os maiores vetores de memória estão em quatro áreas:

1. Serviço de busca: `VolumeIndex::new` reserva capacidade alta fixa para qualquer volume, inclusive volumes pequenos e caminhos de load temporários.
2. App principal: listas grandes de `FileEntry` são clonadas em várias camadas (`all_items`, `items`, tabs, jobs de rebuild), amplificando o custo de cada entrada.
3. Caches/filas do file manager: `DirectoryCache` é limitado por número de pastas, não por entradas/bytes, e a fila de bulk thumbnails pode crescer com a árvore inteira.
4. egui/GPU: `IconLoader` tem mapas de `TextureHandle` sem LRU, enquanto o `CacheManager` principal já é bem limitado.

## Status de implementação

Itens já implementados nesta etapa:

- `IconLoader`: `drive_icon_cache`, `extension_cache` e cache de falhas migrados para LRU com limites explícitos.
- Folder covers: removidos índices temporários `HashMap<PathBuf, usize>` proporcionais ao diretório inteiro durante a aplicação de lotes.
- Bulk thumbnails: produtor do scan em massa agora aplica backpressure via `pending_count()` antes de continuar a varredura.
- `DirectoryCache`: além do limite por número de pastas, agora há orçamento global por total de entradas cacheadas.
- Text viewer: caminho UTF-8 puro agora move `Vec<u8>` para `String` sem cópia extra quando possível.
- Image viewer SVG: validação do limite de 50 MB ocorre antes de duplicar o buffer do arquivo.
- Search service: o caminho de load binário passou a usar índice vazio; MFT, USN e non-USN agora usam capacidade proporcional ou vazia em vez da pré-alocação fixa agressiva quando já existe estimativa ou quando o índice ainda não foi preenchido.
- Search service: após load/scan completo, `VolumeIndex` agora compacta arena e mapas (`HashMap`/`HashSet`) antes de entrar em regime estável.
- Folder size batch state: `batch_invalidation_epoch` agora sofre poda periódica de paths sem cache, request em voo ou revalidação pendente.
- Tabs: snapshots de tabs inativas agora podem descartar `items` quando ele é redundante com `all_items`, reconstruindo a view só ao reativar o tab.
- Dual panel: o estado do painel inativo armazenado dentro de tabs agora é compactado para não carregar `items` redundante quando o tab está fora de foco, sem repetir reconstruções de lista no snapshot vivo do dual panel renderizado a cada frame.
- GIF manager: o decode incremental agora respeita teto por GIF e também interrompe crescimento adicional quando o orçamento global já foi alcançado, mantendo os primeiros frames já decodificados em vez de continuar acumulando memória até a próxima limpeza.
- Rebuild assíncrono de `items`: o painel ativo agora coalesce rebuilds em voo, evitando spawn de múltiplos jobs que clonavam `all_items` repetidamente antes do resultado anterior voltar; mudanças acumuladas durante o job são reagendadas só após a conclusão do rebuild atual.
- Dual panel streaming: rebuilds intermediários do painel inativo agora são throttled/coalescidos durante o streaming, em vez de forçar `filter_items()` a cada lote recebido.
- Mutações pontuais da lista: removidos caminhos que faziam `filter_items()` seguido de `sort_items()`, embora `filter_items()` já produza o resultado ordenado.
- Soft reload por watcher: `stale_items_snapshot` agora captura só paths com estado visual realmente cacheado (texture, RGBA, preview de pasta ou falha), em vez de copiar metadados do diretório inteiro; paths removidos também liberam mais estado associado ao reconciliar o snapshot.
- Sidebar tree: o cache `children` agora poda ramos colapsados que não são mais ancestrais de nós expandidos, reduzindo retenção de subárvores antigas em sessões longas.
- Global search: fechar o overlay ou zerar a consulta agora libera também os vetores derivados (`results`, índices filtrados/ordenados e drives disponíveis) com `shrink_to_fit()`, em vez de só limpar conteúdo lógico e manter a capacidade reservada após buscas grandes.
- Search service USN: o caminho que carrega índice via SQLite agora também chama `shrink_to_fit()` completo, alinhando a compactação de mapas com os demais caminhos de load.

Itens ainda pendentes por maior risco ou necessidade de benchmark:

- Redução estrutural da duplicação entre `all_items`, `items`, tabs e snapshots de dual-panel.
- Rework de load/save binário do search service para streaming/mmap com HMAC incremental.
- Estratégia alternativa para `NameArena.lowered` e para memória de GIFs sem degradar latência/UX.

## Achados de alto impacto

### 🔴 1. Pré-alocação fixa agressiva em `VolumeIndex::new`

**Localização:** [crates/mtt-search-service/src/file_index.rs](crates/mtt-search-service/src/file_index.rs#L107), [crates/mtt-search-service/src/file_index.rs](crates/mtt-search-service/src/file_index.rs#L110-L113), uso em [crates/mtt-search-service/src/volume_indexers/usn.rs](crates/mtt-search-service/src/volume_indexers/usn.rs#L47), [crates/mtt-search-service/src/volume_indexers/non_usn.rs](crates/mtt-search-service/src/volume_indexers/non_usn.rs#L67), [crates/mtt-search-service/src/volume_indexers/non_usn.rs](crates/mtt-search-service/src/volume_indexers/non_usn.rs#L97), [crates/mtt-search-service/src/index_db/binary.rs](crates/mtt-search-service/src/index_db/binary.rs#L360), [crates/mtt-search-service/src/mft_reader.rs](crates/mtt-search-service/src/mft_reader.rs#L1276)

**Descrição objetiva:** todo `VolumeIndex::new` reserva `records` com 500.000 slots, `children` com 200.000 buckets e `NameArena` com 12,5 MB antes de saber o tamanho real do volume. Isso acontece também em fallback non-USN, load binário e testes/caminhos temporários.

**Impacto de memória:** alto para volumes pequenos, removíveis, fallback non-NTFS e ciclos de load/rebuild. Em máquinas com vários volumes, cada índice paga dezenas de MB antes de ter evidência de que precisa disso. Além disso, [crates/mtt-search-service/src/file_index.rs](crates/mtt-search-service/src/file_index.rs#L545) subestima memória porque só estima `records` + arena, sem `children`, `hardlink_parents`, `reparse_points`, `pending_*`, `dir_modified_at` e `NameArena.lowered`.

**Antes:** qualquer volume começa com capacidade equivalente a 500k registros e 12,5 MB de nomes.

**Depois proposto:** criar construtores explícitos, por exemplo `VolumeIndex::empty(drive_letter)` e `VolumeIndex::with_capacity(drive_letter, estimated_records, estimated_name_bytes)`. Usar:

- `record_count` e `arena_size` do binário em [crates/mtt-search-service/src/index_db/binary.rs](crates/mtt-search-service/src/index_db/binary.rs#L330-L362).
- `total_records` do MFT em [crates/mtt-search-service/src/mft_reader.rs](crates/mtt-search-service/src/mft_reader.rs#L1264-L1276).
- contagem persistida do SQLite quando disponível.
- default pequeno para fallback desconhecido, com `reserve` adaptativo.

Também vale adicionar `shrink_maps_to_fit()` após cargas/scans completos quando o tamanho final for muito menor que a capacidade.

**Risco de regressão:** médio. Subestimar capacidade pode causar reallocs durante scan. Mitigação: reservar a partir de contadores já conhecidos nos caminhos críticos e usar default conservador só onde não há estimativa.

**Impacto em performance:** neutro ou positivo no load/cache binário. Pode haver pequena regressão em volumes desconhecidos se a capacidade inicial for pequena demais; deve ser medido com full scan em volume grande.

**Prioridade:** 🔴 Alto impacto.

### 🔴 2. `DirectoryCache` é limitado por pastas, não por quantidade de entradas/bytes

**Localização:** [src/infrastructure/directory_cache.rs](src/infrastructure/directory_cache.rs#L12), [src/infrastructure/directory_cache.rs](src/infrastructure/directory_cache.rs#L15), [src/infrastructure/directory_cache.rs](src/infrastructure/directory_cache.rs#L78-L94), [src/infrastructure/directory_cache.rs](src/infrastructure/directory_cache.rs#L131)

**Descrição objetiva:** o cache guarda até 200 diretórios (`CACHE_CAPACITY`), e cada diretório guarda `Arc<Vec<FileEntry>>`. O código já remove `folder_cover` no `put`, o que é bom, mas não há orçamento por quantidade total de `FileEntry` nem por bytes estimados.

**Impacto de memória:** alto em sessões que visitam muitas pastas grandes. 200 diretórios pequenos são baratos; 200 diretórios com dezenas de milhares de entradas podem manter centenas de MB vivos.

**Antes:** `LruCache<PathBuf, CachedFolder>` limita apenas `entries.len()` do cache.

**Depois proposto:** manter o limite de 200 pastas, mas adicionar orçamento global, por exemplo `max_total_entries` e/ou `max_estimated_bytes`. O método `stats()` já calcula total de itens, então a infraestrutura básica existe. Ao inserir, evictar LRU até ficar abaixo do orçamento.

**Risco de regressão:** baixo a médio. Pastas frias podem precisar ser relidas ao voltar. Mitigação: orçamento alto o bastante para manter o working set normal e evictar primeiro diretórios muito grandes/frios.

**Impacto em performance:** CPU neutra; pode aumentar I/O em navegação de retorno para diretórios evictados. Deve ser medido em HDD e OneDrive.

**Prioridade:** 🔴 Alto impacto.

### 🔴 3. Fila de bulk thumbnails pode crescer com a árvore inteira

**Localização:** fila em [src/workers/thumbnail/queue.rs](src/workers/thumbnail/queue.rs#L14-L17), contador em [src/workers/thumbnail/queue.rs](src/workers/thumbnail/queue.rs#L63), produtor em [src/ui/app/layers/status_bar_layer.rs](src/ui/app/layers/status_bar_layer.rs#L106), enqueue em [src/ui/app/layers/status_bar_layer.rs](src/ui/app/layers/status_bar_layer.rs#L141-L151)

**Descrição objetiva:** `PriorityThumbnailQueue` deduplica por path, mas não tem limite de profundidade. O bulk scan percorre `WalkDir::new(&root)` e chama `queue.push_bulk_scan` para todo arquivo de mídia sem thumbnail em disco. Em árvores grandes, o produtor pode enfileirar muito mais rápido que os workers consomem.

**Impacto de memória:** alto em pastas com muitos vídeos/imagens. Cada item pendente mantém pelo menos um `PathBuf` em `pending`, outro dentro de `ThumbnailRequest`, agrupamento por diretório e overhead de hash/vector. Em centenas de milhares de mídias, isso vira um pico grande antes de qualquer decode.

**Antes:** a fila pode conter todos os arquivos de mídia não cacheados encontrados pela varredura.

**Depois proposto:** aplicar backpressure específico para `BulkScan`: usar `pending_count()` e um limite como `MAX_BULK_THUMBNAIL_PENDING` (ex.: 2.000-10.000, calibrado). O produtor deve pausar/yield enquanto a fila estiver acima do limite e continuar checando `scanning_flag` para cancelamento. Não limitar requisições interativas; elas devem continuar promovendo prioridade como hoje.

**Risco de regressão:** baixo. A varredura em massa fica mais lenta em árvores enormes, mas é trabalho de fundo. A UI fica mais previsível e as thumbnails visíveis continuam prioritárias.

**Impacto em performance:** CPU neutra ou melhor por menor pressão de alocação. Throughput total pode reduzir por backpressure, mas sem afetar responsividade.

**Prioridade:** 🔴 Alto impacto.

### 🔴 4. `IconLoader` mantém mapas de texturas sem LRU

**Localização:** campos em [src/ui/icon_loader.rs](src/ui/icon_loader.rs#L97-L101), inicialização em [src/ui/icon_loader.rs](src/ui/icon_loader.rs#L131-L139), clear parcial em [src/ui/icon_loader.rs](src/ui/icon_loader.rs#L151), inserts em [src/ui/icon_loader/file_icons.rs](src/ui/icon_loader/file_icons.rs#L347), [src/ui/icon_loader/async_ops.rs](src/ui/icon_loader/async_ops.rs#L61-L66), contraste com cache já limitado em [src/ui/cache.rs](src/ui/cache.rs#L77-L93)

**Descrição objetiva:** `icon_cache` é `LruCache` com 512 entradas, mas `drive_icon_cache`, `failed_drive_icons` e `extension_cache` são `HashMap`/`HashSet` sem limite. Além disso, `clear()` não limpa `extension_cache`, apesar do comentário dizer que limpa caches de ícones.

**Impacto de memória:** alto para sessões longas com muitos tipos de arquivo, extensões incomuns, ícones Jumbo e special folders. Como os valores são `egui::TextureHandle`, há impacto em memória de GPU/driver além do heap Rust.

**Antes:** crescimento monotônico por chave distinta em `extension_cache` e `drive_icon_cache`; falhas também ficam sem limite.

**Depois proposto:** trocar `drive_icon_cache` e `extension_cache` para `LruCache<String, TextureHandle>` com limites explícitos (ex.: 64 para drive/special, 512 para extensões). Trocar `failed_drive_icons` para LRU/set com capacidade. Fazer `clear()` limpar `extension_cache` ou renomear o método se a persistência for intencional.

**Risco de regressão:** baixo. Evictar ícone frio causa recarregamento posterior, sem alterar comportamento visual permanente.

**Impacto em performance:** neutro. Pode haver pequenas recargas de ícones raros, compensadas por menor pressão de GPU/heap.

**Prioridade:** 🔴 Alto impacto.

### 🔴 5. Clonagem pesada de listas `FileEntry` no estado principal, tabs e rebuilds

**Localização:** estado em [src/app/state/mod.rs](src/app/state/mod.rs#L92), [src/app/state/mod.rs](src/app/state/mod.rs#L186), campos de `FileEntry` em [src/domain/file_entry.rs](src/domain/file_entry.rs#L24-L31), clones em [src/app/operations/folder_loading/view_updates.rs](src/app/operations/folder_loading/view_updates.rs#L27), [src/app/operations/message_handler/thumbnail_rebuild.rs](src/app/operations/message_handler/thumbnail_rebuild.rs#L55), [src/app/operations/message_handler/thumbnail_rebuild.rs](src/app/operations/message_handler/thumbnail_rebuild.rs#L80), tab sync em [src/app/operations/tabs.rs](src/app/operations/tabs.rs#L19-L45), duplicate tab em [src/tabs/mod.rs](src/tabs/mod.rs#L354), dual-panel snapshot inicial em [src/app/dual_panel.rs](src/app/dual_panel.rs#L106)

**Descrição objetiva:** o app mantém `all_items: Vec<FileEntry>` como cache mestre e `items: Arc<Vec<FileEntry>>` como view renderizada. Em vários pontos a view é recriada via clone completo. Tabs também mantêm `all_items`, e `sync_to_tab()` clona a lista ativa para o tab. `FileEntry` inclui `PathBuf`, `String`, `Option<PathBuf>` e `Option<DriveInfo>`, então cada clone duplica buffers de heap.

**Impacto de memória:** alto em diretórios grandes. O estado pode manter cópias simultâneas: mestre, view ordenada/filtrada, tab ativo, job de rebuild em thread e snapshots de painel/tab. O dual-panel usa `swap_with_app()` para troca normal, o que é bom; o problema mais forte está em sync/rebuild/list view e criação/duplicação de snapshots.

**Antes:** `Vec<FileEntry>` completo para mestre e views; clones em rebuild e tab sync.

**Depois proposto:** migrar gradualmente para uma das opções:

- Mestre `Arc<[FileEntry]>` e view como `Vec<usize>`/`Arc<[usize]>` ordenada/filtrada.
- Estado ativo único: app ou tab possui `all_items`, não ambos ao mesmo tempo; mover em troca de tab em vez de clonar.
- Para mudanças menores, eliminar clones desnecessários em `sync_to_tab()` quando o tab ativo já representa o próprio estado, e mover o snapshot somente ao desativar.

Medir `std::mem::size_of::<FileEntry>()` e considerar mover campos raros (`drive_info`, `folder_cover`) para side maps ou `Box` somente se o tamanho real justificar.

**Risco de regressão:** alto. Seleção, ordenação, busca local, tabs, dual panel e watcher dependem da semântica atual. Precisa de testes focados em navegação, troca de tabs, dual panel, busca e refresh.

**Impacto em performance:** pode melhorar CPU por reduzir clones, mas views por índice exigem cuidado para não piorar cache locality e sorting. Não implementar sem benchmark.

**Prioridade:** 🔴 Alto impacto.

## Achados de médio impacto

### 🟡 6. Load/save binário duplica picos grandes do índice

**Localização:** save payload em [crates/mtt-search-service/src/index_db/binary.rs](crates/mtt-search-service/src/index_db/binary.rs#L103-L111), HMAC sobre payload em [crates/mtt-search-service/src/index_db/binary.rs](crates/mtt-search-service/src/index_db/binary.rs#L177), load inteiro em [crates/mtt-search-service/src/index_db/binary.rs](crates/mtt-search-service/src/index_db/binary.rs#L211), arena copiada em [crates/mtt-search-service/src/index_db/binary.rs](crates/mtt-search-service/src/index_db/binary.rs#L362)

**Descrição objetiva:** `save()` constrói `payload: Vec<u8>` contendo header, arena, records e reparse points antes de escrever, além de `hardlink_pairs`, `sorted_frns` e `sorted_reparse`. `load()` lê o arquivo inteiro em `Vec<u8>` e depois copia a arena para `NameArena::from_raw`.

**Impacto de memória:** médio a alto durante persist/load de volumes grandes. É transitório, mas pode coincidir com full scan e `NameArena.lowered`, elevando bastante o pico do serviço.

**Antes:** arquivo binário inteiro fica materializado em memória no save e no load.

**Depois proposto:** usar HMAC incremental/streaming ou `memmap2` para validar sem copiar todo o arquivo. No save, escrever chunks em arquivo temporário enquanto alimenta HMAC, preservando trailer HMAC. No load, validar via mmap ou streaming e construir `VolumeIndex` diretamente com capacidades do header.

**Risco de regressão:** médio. A integridade HMAC é parte de segurança; refatorar exige testes de corrupção, legacy magic, mismatch de drive, truncamento e replay de arquivos antigos.

**Impacto em performance:** tende a melhorar por menos cópia e menos pressão no allocator. HMAC continua O(n).

**Prioridade:** 🟡 Médio impacto.

### 🟡 7. Atualizações de folder cover criam índices temporários com clones de `PathBuf`

**Localização:** [src/app/operations/message_handler/thumbnail_workers.rs](src/app/operations/message_handler/thumbnail_workers.rs#L63-L68), [src/app/operations/message_handler/thumbnail_workers.rs](src/app/operations/message_handler/thumbnail_workers.rs#L95-L100)

**Descrição objetiva:** a cada lote de covers, o código cria `HashMap` com capacidade de `self.all_items.len()` e insere `item.path.clone()` para todos os itens; depois repete o mesmo para `items`. O lote processado é limitado, mas o índice temporário é proporcional ao diretório inteiro.

**Impacto de memória:** médio em diretórios grandes, especialmente quando chegam múltiplos lotes de cover worker. É memória transitória por frame, mas pode gerar spikes e pressão de allocator no thread da UI.

**Antes:** aloca dois `HashMap<PathBuf, usize>` grandes e clona todos os paths.

**Depois proposto:** inverter o loop: iterar `all_items`/`items` uma vez e consultar `cover_updates.get(&item.path)`. Isso evita clones de `PathBuf` e mantém complexidade O(n) similar. Outra opção é usar índices emprestados só até coletar os índices e dropar o mapa antes da mutação.

**Risco de regressão:** baixo. A lógica de atualização é a mesma.

**Impacto em performance:** deve melhorar CPU e memória; não há perda de UX esperada.

**Prioridade:** 🟡 Médio impacto.

### 🟡 8. `GifManager` principal tem orçamento, mas decode pode ultrapassá-lo antes da limpeza

**Localização:** orçamento em [src/ui/components/gif_manager.rs](src/ui/components/gif_manager.rs#L63-L66), canal limitado em [src/ui/components/gif_manager.rs](src/ui/components/gif_manager.rs#L74), limite configurado em [src/ui/components/gif_manager.rs](src/ui/components/gif_manager.rs#L99-L103), cleanup em [src/ui/components/gif_manager.rs](src/ui/components/gif_manager.rs#L177), push de frames em [src/ui/components/gif_manager.rs](src/ui/components/gif_manager.rs#L250-L253), limite de frames em [src/ui/components/gif_manager.rs](src/ui/components/gif_manager.rs#L272)

**Descrição objetiva:** o manager tem `max_memory_bytes = 150 MB` e LRU de 100 GIFs, mas o worker adiciona frames e incrementa `running_total_bytes` durante decode. A limpeza roda em chamadas de `cleanup`, não como hard stop dentro do worker. Um GIF atual grande pode ultrapassar o orçamento até 500 frames redimensionados para 512 px.

**Impacto de memória:** médio. O impacto é menor que no viewer de imagens standalone, mas previews GIF no app principal podem manter muitos RGBA frames em heap.

**Antes:** o limite é aplicado por limpeza externa; o decode em andamento pode passar do orçamento.

**Depois proposto:** não truncar GIF silenciosamente. Primeiro medir. Se confirmado, preferir uma estratégia que preserve UX: orçamento por GIF com representação alternativa, cache de frames comprimidos/spill temporário, ou carregar somente o GIF ativo e cancelar decodes não ativos antes de crescer. Se for aceitável alinhar comportamento ao viewer standalone, aplicar cap explícito documentado.

**Risco de regressão:** médio a alto se a solução truncar animações ou mudar playback. Por isso é medição antes de alteração.

**Impacto em performance:** soluções de streaming/on-demand podem aumentar CPU; evitar sem benchmark.

**Prioridade:** 🟡 Médio impacto.

### 🟡 9. `NameArena.lowered` dobra os bytes de nomes para acelerar busca

**Localização:** campo em [crates/mtt-search-service/src/name_arena.rs](crates/mtt-search-service/src/name_arena.rs#L31-L33), construção em [crates/mtt-search-service/src/name_arena.rs](crates/mtt-search-service/src/name_arena.rs#L110-L118), chamada em [crates/mtt-search-service/src/volume_indexers/usn.rs](crates/mtt-search-service/src/volume_indexers/usn.rs#L358)

**Descrição objetiva:** após o índice ficar pronto, `build_lowered()` clona todo `buf` para `lowered` e aplica lowercase ASCII. Isso é uma duplicação deliberada para busca SIMD case-insensitive sem alocação por query.

**Impacto de memória:** médio a alto proporcional ao total de bytes de nomes por volume.

**Antes:** `buf` + `lowered` ficam residentes quando o volume está pronto.

**Depois proposto:** não remover como quick win. Medir latência de busca e working set. Só considerar lazy build após primeira busca, build por volume sob demanda, ou representação compacta se o impacto de idle memory for mais importante que primeira busca. Expor `lowered.len()/capacity()` no `memory_usage()` para visibilidade.

**Risco de regressão:** alto se removido ou tornado lazy sem UX guard, porque pode piorar primeira busca e queries interativas.

**Impacto em performance:** manter como está favorece CPU/latência. Otimizar memória aqui provavelmente troca memória por CPU/latência.

**Prioridade:** 🟡 Médio impacto, medição obrigatória.

## Achados de baixo impacto

### 🟢 10. Text viewer duplica bytes no caminho UTF-8 comum

**Localização:** limite de arquivo em [src/text_viewer/mod.rs](src/text_viewer/mod.rs#L21), read em [src/text_viewer/viewer_app.rs](src/text_viewer/viewer_app.rs#L87), decode em [src/text_viewer/viewer_app.rs](src/text_viewer/viewer_app.rs#L652-L663), line offsets em [src/text_viewer/viewer_app.rs](src/text_viewer/viewer_app.rs#L105)

**Descrição objetiva:** `new()` lê `raw: Vec<u8>` e `decode_text(&raw)` converte UTF-8 válido via `std::str::from_utf8(raw)` seguido de `s.to_string()`, duplicando o buffer durante a carga. O limite de 25 MB reduz o risco.

**Impacto de memória:** baixo a médio e transitório. Pico aproximado no caminho UTF-8: `raw` + `String` + `line_offsets`.

**Antes:** UTF-8 válido copia `raw` para novo `String`.

**Depois proposto:** mudar `decode_text` para receber `Vec<u8>` e usar `String::from_utf8(raw)` no caminho UTF-8 sem BOM. Para BOM, remover os três bytes antes de mover ou aceitar uma cópia pequena. Windows-1252 continuará alocando `String`, como esperado.

**Risco de regressão:** baixo, mas precisa testar UTF-8 BOM, UTF-8 puro, Windows-1252 e detecção binária.

**Impacto em performance:** positivo ou neutro, por remover cópia.

**Prioridade:** 🟢 Baixo impacto.

### 🟢 11. SVG é copiado antes da checagem de limite de 50 MB

**Localização:** [src/image_viewer/loader.rs](src/image_viewer/loader.rs#L381-L391), render thread em [src/image_viewer/loader.rs](src/image_viewer/loader.rs#L398-L405)

**Descrição objetiva:** `decode_svg_frame` lê via `read_file_fast`, chama `bytes.as_slice().to_vec()` e só depois rejeita SVG acima de 50 MB. Para arquivos mapeados, isso duplica o arquivo antes de rejeitar.

**Impacto de memória:** baixo a médio, SVG específico e limitado a 50 MB, mas é um quick win claro.

**Antes:** `to_vec()` ocorre antes do size guard.

**Depois proposto:** checar `bytes.as_slice().len()` antes de `to_vec()`. Se possível, passar `Arc<[u8]>` para o worker em vez de copiar quando a origem já puder ser compartilhada com segurança.

**Risco de regressão:** baixo.

**Impacto em performance:** positivo ou neutro.

**Prioridade:** 🟢 Baixo impacto.

### 🟢 12. `batch_invalidation_epoch` pode crescer em sessões longas

**Localização:** campo em [src/app/folder_size_state.rs](src/app/folder_size_state.rs#L107-L108), bump em [src/app/operations/message_handler/helpers.rs](src/app/operations/message_handler/helpers.rs#L347-L357), leitura em [src/app/operations/ui_rendering/list_bridge.rs](src/app/operations/ui_rendering/list_bridge.rs#L361-L370), revalidation em [src/app/operations/message_handler/thumbnail_workers.rs](src/app/operations/message_handler/thumbnail_workers.rs#L617-L628)

**Descrição objetiva:** `pending_revalidation` tem poda, e os caches de folder size são LRU, mas `batch_invalidation_epoch` é `HashMap<PathBuf, u64>` sem limpeza explícita. Cada pasta invalidada pode deixar uma chave permanente.

**Impacto de memória:** baixo na maioria das sessões; pode crescer em sessões muito longas com muita atividade de watcher em muitas pastas distintas.

**Antes:** chave permanece após cache/revalidation expirar.

**Depois proposto:** podar epochs para paths que não estão em `batch_loading`, `pending_revalidation`, `batch_cache` ou `cache`. Fazer isso em cadence baixa junto da poda de revalidations.

**Risco de regressão:** baixo se a poda preservar paths com requests em voo. Não remover epoch enquanto houver `batch_loading` ou pending request.

**Impacto em performance:** neutro.

**Prioridade:** 🟢 Baixo impacto.

## Pontos já bem controlados

- Thumbnails principais: [src/ui/cache.rs](src/ui/cache.rs#L77-L93) usa LRU para texturas, previews de pasta e RGBA, e [src/ui/cache.rs](src/ui/cache.rs#L260-L280) faz trimming por orçamento.
- Upload de thumbnails: [src/app/operations/message_handler/thumbnail_uploads.rs](src/app/operations/message_handler/thumbnail_uploads.rs#L7) define limite de pendentes e o pipeline aplica orçamento por frame.
- Workers de thumbnails: [src/workers/thumbnail/worker.rs](src/workers/thumbnail/worker.rs#L27), [src/workers/thumbnail/worker.rs](src/workers/thumbnail/worker.rs#L90-L118) limitam decode concorrente a no máximo 4.
- Image viewer standalone: [src/image_viewer/cache.rs](src/image_viewer/cache.rs#L95), [src/image_viewer/cache.rs](src/image_viewer/cache.rs#L127-L129), [src/image_viewer/loader.rs](src/image_viewer/loader.rs#L98), [src/image_viewer/loader.rs](src/image_viewer/loader.rs#L153-L196) mostram janela de cache, canais bounded e limites para display/GIF.
- PDF viewer: [src/pdf_viewer/viewer_app.rs](src/pdf_viewer/viewer_app.rs#L18-L25), [src/pdf_viewer/viewer_app.rs](src/pdf_viewer/viewer_app.rs#L245-L276) têm raio e orçamento de textura de 128 MB, com eviction de `page_text` junto das texturas.
- Global search UI: [src/app/global_search_state.rs](src/app/global_search_state.rs#L104-L110) usa LRUs, e o clone de paths para sort por metadata é limitado por [src/app/global_search_state.rs](src/app/global_search_state.rs#L8), [src/app/global_search_state.rs](src/app/global_search_state.rs#L192-L200).
- Navegação/dual panel: troca normal de painel usa `swap` em [src/app/dual_panel.rs](src/app/dual_panel.rs#L190), então não é uma clonagem a cada switch.

## Quick wins recomendados

1. Trocar `IconLoader.drive_icon_cache`, `IconLoader.extension_cache` e failed sets por LRUs com capacidade.
2. Adicionar backpressure para `push_bulk_scan` usando `pending_count()`.
3. Adicionar orçamento por total de entradas/bytes em `DirectoryCache`.
4. Criar `VolumeIndex::with_capacity` e remover `VolumeIndex::new` de caminhos que já conhecem `record_count`, `arena_size` ou `total_records`.
5. Remover clones de `PathBuf` nos índices temporários de folder cover.
6. Mover `Vec<u8>` para `String` no text viewer UTF-8 e checar tamanho SVG antes de `to_vec()`.

## Pontos de profiling antes/depois

### Métricas internas a adicionar em log debug

- `std::mem::size_of::<FileEntry>()` e contagem de `all_items`, `items`, tabs abertos, snapshots de dual panel.
- `DirectoryCache::stats()` mais estimativa de bytes por entrada.
- Tamanhos de `IconLoader`: `icon_cache.len()`, `extension_cache.len()`, `drive_icon_cache.len()`, failed sets.
- `PriorityThumbnailQueue::pending_count()` separado por `Normal` vs `BulkScan`.
- `VolumeIndex::memory_usage()` expandido: `records.capacity`, `children.capacity`, soma de `children` vectors, `names.len/capacity`, `lowered.len/capacity`, `hardlink_parents`, `reparse_points`, `pending_*`.
- Pico de `binary::load` e `binary::save`: payload bytes, file bytes, arena bytes, record count.

### Cenários de medição

1. App frio, sem navegar.
2. Abrir pasta com 10k, 50k e 100k entradas.
3. Navegar por 50-200 pastas grandes para medir `DirectoryCache`.
4. Rodar bulk thumbnail scan em árvore grande com cache vazio.
5. Abrir diretório com muitas extensões raras e special folders para medir `IconLoader`.
6. Serviço de busca: cold load binário, full MFT scan, fallback non-USN em volume pequeno e grande.
7. Selecionar GIF grande no app principal.
8. Abrir texto UTF-8 de 25 MB e SVG perto/acima de 50 MB.

### Ferramentas sugeridas no Windows

- VMMap para separar private bytes, mapped files, heap e GPU/driver mappings por processo.
- Windows Performance Recorder/Analyzer com heap provider para picos durante load/save do índice.
- Visual Studio Diagnostic Tools para snapshots de heap do app GUI e serviço.
- Logs internos com working set por processo em pontos de transição, sem probes agressivos.

## Ordem sugerida de implementação

1. `IconLoader` LRU + clear correto: baixo risco, ganho direto em GPU/heap.
2. Backpressure de bulk thumbnails: baixo risco e protege cenários extremos.
3. `DirectoryCache` com orçamento total: ganho alto, risco moderado de I/O em revisitas.
4. `VolumeIndex::with_capacity` e construção direta no load binário: ganho alto no serviço.
5. Otimizações transitórias pequenas: cover update sem clones, text viewer move UTF-8, SVG size guard.
6. Redesenho de `FileEntry`/views por índice: maior ganho potencial, mas maior risco; fazer só com benchmark e testes de navegação/tabs/dual panel.
