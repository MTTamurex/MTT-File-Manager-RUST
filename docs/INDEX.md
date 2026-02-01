# Índice de Documentação - MTT File Manager

Este índice fornece navegação para todos os documentos técnicos do MTT File Manager.

## Documentos Principais

### [01_overview.md](01_overview.md)
**Visão Geral do Projeto**
- O que é o MTT File Manager
- Principais recursos e capacidades
- Arquitetura de alto nível
- Tecnologias utilizadas
- Limitações conhecidas

### [02_build_run_debug.md](02_build_run_debug.md)
**Build, Execução e Debug**
- Pré-requisitos e instalação
- Como compilar (dev e release)
- Como executar com logs
- Debug e profiling
- Solução de problemas comuns

### [03_architecture.md](03_architecture.md)
**Arquitetura do Sistema**
- Camadas e responsabilidades
- Principais boundaries
- Ciclo de vida da aplicação
- Estado global e gerenciamento
- Pontos de extensão

### [04_module_map.md](04_module_map.md)
**Mapa do Repositório**
- Estrutura de diretórios completa
- Responsabilidades por módulo
- Principais structs e funções
- Dependências entre módulos
- Fluxo de dados principal

### [05_dependencies_stack.md](05_dependencies_stack.md)
**Stack de Dependências**
- Dependências do Cargo.toml
- Features e versionamento
- Integrações com Windows
- Alternativas consideradas
- Compatibilidade

### [06_key_flows.md](06_key_flows.md)
**Fluxos Principais**
- Navegação para pasta
- Preview de arquivo
- Operações de arquivo
- Geração de thumbnail
- Menu de contexto
- Debugging por fluxo

### [07_storage_config.md](07_storage_config.md)
**Configuração e Storage**
- Localização dos arquivos
- Banco de dados SQLite
- Configurações e preferências
- Cache de thumbnails
- Como resetar/limpar dados

### [08_logging_errors_telemetry.md](08_logging_errors_telemetry.md)
**Logs, Erros e Telemetria**
- Sistema de logs atual
- Como capturar logs
- Sistema de erros AppError
- Stack traces e backtraces
- Debugging avançado

### [09_support_playbook.md](09_support_playbook.md)
**Playbook de Suporte**
- Checklist de triagem
- Problemas comuns e soluções
- Perguntas padrão para tickets
- Scripts de diagnóstico
- Procedimentos de suporte

## Navegação Rápida por Tópico

### Para Novos Desenvolvedores
1. Comece com [01_overview.md](01_overview.md) para entender o que é
2. Veja [02_build_run_debug.md](02_build_run_debug.md) para configurar ambiente
3. Leia [03_architecture.md](03_architecture.md) para entender a arquitetura
4. Use [04_module_map.md](04_module_map.md) para navegar o código

### Para Debug e Suporte
1. Use [09_support_playbook.md](09_support_playbook.md) para triagem inicial
2. Veja [08_logging_errors_telemetry.md](08_logging_errors_telemetry.md) para capturar logs
3. Consulte [06_key_flows.md](06_key_flows.md) para debugging por fluxo
4. Use [07_storage_config.md](07_storage_config.md) para problemas de cache/config

### Para Entender Dependências
1. Leia [05_dependencies_stack.md](05_dependencies_stack.md) para stack completo
2. Verifique [04_module_map.md](04_module_map.md) para dependências por módulo

### Para Problemas de Performance
1. Use [08_logging_errors_telemetry.md](08_logging_errors_telemetry.md) para métricas
2. Veja [06_key_flows.md](06_key_flows.md) para pontos de performance
3. Consulte [07_storage_config.md](07_storage_config.md) para cache optimization

## Arquivos de Código Importantes

### Entry Points
- `src/main.rs` - Ponto de entrada do aplicativo
- `src/lib.rs` - Ponto de entrada da biblioteca
- `src/app/state.rs` - Estado principal da aplicação

### Fluxos Principais
- `src/app/operations/navigation.rs` - Navegação
- `src/app/operations/folder_loading.rs` - Carregamento de pastas
- `src/app/operations/thumbnails.rs` - Thumbnails
- `src/workers/thumbnail_worker.rs` - Workers de thumbnail

### Integrações Críticas
- `src/infrastructure/windows/` - Integrações Windows
- `src/infrastructure/disk_cache.rs` - Cache em disco
- `src/ui/app_impl.rs` - Main loop da UI

## Comandos Úteis

### Build e Execução
```bash
# Desenvolvimento
cargo build
cargo run

# Produção
cargo build --release
cargo run --release

# Com logs
.\target\release\mtt-file-manager.exe 2>&1 | Tee-Object "debug.log"
```

### Debug e Testes
```bash
# Executar benchmarks
cargo bench

# Verificar dependências
cargo tree
cargo audit

# Formatar código
cargo fmt

# Lint
cargo clippy
```

### Limpeza e Reset
```powershell
# Limpar cache
Remove-Item "$env:LOCALAPPDATA\MTT-File-Manager" -Recurse -Force

# Limpar build
cargo clean

# Limpar tudo
cargo clean
Remove-Item "$env:LOCALAPPDATA\MTT-File-Manager" -Recurse -Force
```

## Status do Projeto

✅ **Documentação Completa** - Todos os documentos principais criados  
✅ **Arquitetura Documentada** - Camadas e fluxos descritos  
✅ **Dependências Mapeadas** - Stack completa documentada  
✅ **Fluxos Detalhados** - Principais fluxos documentados  
✅ **Playbook de Suporte** - Procedimentos de suporte definidos  

## Notas e Limitações

### Documentação Futura (Não Implementada)
- API pública (não existe atualmente)
- Plugins/extensions (não implementado)
- Temas customizáveis (não implementado)
- Internacionalização (apenas PT-BR)

### Áreas que Precisam de Atenção
- Testes automatizados são mínimos
- Documentação de APIs internas poderia ser mais detalhada
- Guias de contribuição poderiam ser adicionados
- Documentação de deployment/instalação

## Links Externos Importantes

### Rust e Crates
- [Rust Documentation](https://doc.rust-lang.org/)
- [egui Documentation](https://docs.rs/egui/)
- [windows-rs Documentation](https://microsoft.github.io/windows-docs-rs/)

### Windows APIs
- [Windows API Documentation](https://docs.microsoft.com/windows/win32/)
- [Windows Shell Documentation](https://docs.microsoft.com/windows/win32/shell/)
- [Media Foundation](https://docs.microsoft.com/windows/win32/medfound/)

### Dependências Externas
- [libmpv Documentation](https://mpv.io/manual/master/)
- [WebView2 Documentation](https://docs.microsoft.com/microsoft-edge/webview2/)

---

*Esta documentação foi gerada automaticamente baseada na análise do código atual. Mantenha atualizada conforme o projeto evolui.*