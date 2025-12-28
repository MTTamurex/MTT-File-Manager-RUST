# POC Viewer

Visualizador de imagens e vídeos de **ultra-performance** em Rust usando **API nativa do Windows** (`IShellItemImageFactory`).

🎯 **4.3 MB** executável | ⚡ **Zero dependências externas** | 🚀 **Performance nativa do Windows**

## 🎯 Funcionalidades

- ✅ **Thumbnails de Imagens**: PNG, JPG, JPEG, BMP, GIF
- ✅ **Thumbnails de Vídeos**: MP4, MKV, AVI, MOV, WEBM, FLV
- ✅ **Carregamento Assíncrono**: Interface nunca trava
- ✅ **Processamento Paralelo**: Rayon para múltiplas threads
- ✅ **Otimização de GPU**: Thumbnails 256x256px antes do upload
- ✅ **Grid Responsivo**: Ajusta automaticamente ao tamanho da janela

## 📦 Pré-requisitos

1. **Rust**: Instale via [rustup.rs](https://rustup.rs/)
2. **Windows 10/11**: API nativa do Windows

**Sem dependências externas!** Tudo funciona out-of-the-box.

## 🚀 Execução

```powershell
# Compilar e executar (recomendado)
cargo run --release

# Ou executar o binário direto
.\target\release\poc-viewer.exe
```

## 🏗️ Build

```powershell
cargo build --release
```

O executável será gerado em `target\release\poc-viewer.exe`.

## 📝 Como Usar

1. Ao abrir, o visualizador tentará carregar `C:\Users\Public\Pictures`
2. Clique em **"📁 Escolher Pasta"** para selecionar outra pasta
3. Aguarde o carregamento instantâneo - thumbnails aparecem progressivamente
4. Suporta **qualquer formato que o Windows Explorer suporta**

## 🔧 Arquitetura

- **Thread Background (Rayon)**: Processa arquivos em paralelo
- **Windows IShellItemImageFactory**: API nativa para thumbnails (mesmo do Explorer)
- **COM Thread-Safe**: Cada thread rayon inicializa/finaliza COM
- **HBITMAP → RGBA**: Converte formato Windows (BGRA) para egui (RGBA)
- **mpsc Channel**: Envia thumbnails prontas para a UI
- **eframe (egui)**: Renderiza grid responsivo com 60/144fps

## ⚙️ Otimizações

- **API nativa do Windows**: Zero subprocess overhead
- **Cache do sistema**: Windows mantém thumbnails em cache
- **Hardware acceleration**: GPU usada automaticamente quando disponível
- **Processamento paralelo**: Rayon processa múltiplos arquivos simultaneamente
- **Executável tiny**: Apenas 4.3 MB

## 📄 Licença

MIT
