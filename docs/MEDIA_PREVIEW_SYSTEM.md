# 📺 Media Preview System (Miniplayer)

The MTT File Manager features a high-performance, theme-consistent media preview system located in the right sidebar. This system allows for real-time playback of videos, GIFs, and image previews without leaving the file manager.

## 🏗️ Architecture

The miniplayer uses a **Hybrid Architecture** to embed MPV as a native child window inside the right sidebar.

### 1. MPV Engine (`libmpv2`)
- **Reasoning**: Native `egui` does not support hardware-accelerated video decoding directly in a texture without significant performance overhead.
- **Implementation**: We embed an MPV child window via the `wid` property.
- **Visibility**: The MPV window is only shown when a video is active, replacing the static thumbnail.

## 🎨 UI & Controls

While the video is rendered by MPV, the **Controls** are rendered in pure **egui** to ensure visual consistency with the rest of the application.

### State-Aware Components
- **Thumbnail Mode**: Displays a centered Play button overlay on hover.
- **Player Mode**: Active video with a dedicated controls bar immediately below the media area.

### Theme Synchronization
- **Colors**: Uses `COLOR_ACCENT` (App Blue) for the seek bar and volume selection. 
- **Dark/Light Mode**: Icons automatically toggle between dark gray (`#3C3C3C`) and light gray/white based on the global application theme.
- **Visual Polish**: No frames or separators cut through the media area, providing a clean "integrated" look.

## 🛠️ Technical Specs

| Feature | Implementation |
| :--- | :--- |
| **Decoding** | GPU-accelerated via MPV |
| **Streaming** | Direct file playback via MPV |
| **Controls** | Custom egui Widgets |
| **Styles** | Theme-aligned (egui) |
| **Performance** | Direct decode/playback by MPV |
| **Metadata** | 4-Layer Hybrid (Cache -> Registry -> MF -> Sniffing) |

## ⚠️ Known Constraints
- **Native Window Precedence**: Being a native child window, the MPV window always stays on top of any `egui` painting in its area. This is why controls are placed **below** the video instead of as an overlay during active playback.

## 🧩 MPV Runtime

- **Renderização**: MPV embutido como janela filha (`wid`)
- **Controles**: permanecem em egui
- **Formato**: suporte amplo a containers/codecs (geralmente dispensa transcoding)
- **Runtime**: requer `mpv-1.dll` ao lado do executável (ou no PATH)

