# 📺 Media Preview System (Miniplayer)

The MTT File Manager features a high-performance, theme-consistent media preview system located in the right sidebar. This system allows for real-time playback of videos, GIFs, and image previews without leaving the file manager.

## 🏗️ Architecture

The miniplayer uses a **Hybrid Architecture** to overcome the limitations of rendering hardware-accelerated video within a pure Rust/egui GUI on Windows.

### 1. Webview Engine (`wry`)
- **Reasoning**: Native `egui` does not support hardware-accelerated video decoding (H.264/H.265) directly in a texture without significant performance overhead and manual codec implementation.
- **Implementation**: We use the `wry` crate to embed a native Edge/WebView2 window into the sidebar.
- **Visibility**: The WebView is only initialized and visible when a compatible video file is selected, replacing the static thumbnail.

### 2. Local HTTP Streaming Server
- **Protocol**: `http://127.0.0.1:[RANDOM_PORT]`
- **Streaming**: A lightweight local server provides the video file to the WebView using **Range Requests**. This ensures:
    - Instant seeking (Fast-Forward/Rewind).
    - Support for large 4K files without loading the entire file into memory.
    - Compatibility with standard HTML5 `<video>` tags.

### 3. IPC Communication (Bridge)
- **Rust -> JS**: Commands like `play()`, `pause()`, `seek(time)`, and `setVolume(v)` are sent from Rust to the WebView via string scripts.
- **JS -> Rust**: A heartbeat and event system (`window.ipc.postMessage`) sends the current playback state (time, duration, playing status) back to Rust every 16ms to synchronize the UI.

## 🎨 UI & Controls

While the video is rendered by WebView, the **Controls** are rendered in pure **egui** to ensure visual consistency with the rest of the application.

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
| **Decoding** | GPU-accelerated via OS (WebView2/Edge) |
| **Streaming** | `tiny_http` with Byte-Range support |
| **Controls** | Custom egui Widgets |
| **Styles** | CSS-injected for background matching (`#FDFDFD` / `#2D2D2D`) |
| **Performance** | Zero-copy streaming from disk to WebView |

## ⚠️ Known Constraints
- **Native Window Precedence**: Being a native child window, the WebView window always stays on top of any `egui` painting in its area. This is why controls are placed **below** the video instead of as an overlay during active playback.
