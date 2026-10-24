# MTT File Manager

**Native Windows file manager** built in Rust with a modern UI, disk usage analysis, archive browsing and creation, advanced media preview, and Windows integration.

<img width="3840" height="2064" alt="image" src="https://github.com/user-attachments/assets/98196918-bfee-4d40-8fa5-c1ff2914b99d" />

## Download & Install (Windows)

**[Download the latest release](../../releases/latest)**

1. On the release page, expand **Assets** and download **`MTT-File-Manager-Setup-<version>.exe`**. The **Source code** ZIP/TAR files are for developers, not installers.
2. Open the downloaded installer and approve the Windows administrator prompt. Administrator access is needed to install the background search and drive-information service; normal use of the file manager does not require it.
3. If the installer shows **Required Microsoft component**, click **Download from Microsoft**, open the downloaded Visual C++ Redistributable **x64** installer, and finish its installation. Restart Windows first if requested, then return to MTT setup and click **Check again** and **Next**.
4. Finish setup and open **MTT File Manager** from the Start menu. Global Search builds its index in the background; its initial results may be incomplete until indexing finishes.

### Installation help

- **Offline installation:** download the [Microsoft Visual C++ Redistributable x64](https://aka.ms/vs/17/release/vc_redist.x64.exe) beforehand, install it on the target computer, then run MTT setup.
- **Microsoft component still missing:** complete its installation, including a restart if requested, and click **Check again**. Installing the x86 version alone does not satisfy the x64 requirement.
- **An older MTT installer only displays a prerequisite warning:** install the Microsoft component using the link above, then run setup again.
- **Updating:** MTT checks normal GitHub releases at startup and shows the release notes. Use **Settings > About > Check for updates** to check manually, disable startup checks, or enable optional startup installation (off by default). Downloaded installers are verified against GitHub's SHA-256 digest. Updates wait for file operations to finish and other MTT windows/viewers to close; pause enabled Organizer rules before proceeding. Windows requests administrator approval, then MTT reopens without elevation. Setup rebuilds the search index as it already does for manual upgrades. You can still close MTT and run the newer installer manually; no uninstall is needed. Versions released before the built-in updater require one manual upgrade. See [Privacy](#privacy) and the [update release requirements and validation checklist](installer/UPDATES.md).

## Key Features

### Interface & Navigation
- **Appearance themes** — Choose Light, Dark, Dark Deep, Midnight, or System in Settings > Appearance; saved between sessions and shared with image, PDF, DOCX, XLSX, and text viewers, including native title bar styling
- **Interface fonts and text size** — Choose an installed Windows font in **Settings > Appearance**, preview it before confirming, and save a separate text-size preference for each font without resizing icons or document contents
- **Text and interface scaling** — Adjust the status-bar interface-size slider from 100–200%; release it to apply and save the size. Text and controls grow together on top of Windows display scaling; 100% follows Windows scaling alone. Thumbnail size remains separately adjustable alongside it
- **Dual panel (split view)** — Side-by-side file browsing with independent left and right panels; toggle via the toolbar button. Each panel maintains its own navigation history, sort order, view mode, and selection. File copy/move operations default to the opposite panel as the destination
- **Tabbed navigation** — Multiple tabs with independent history; dual-panel mode also provides separate tabs for each pane, with independent navigation state and drag-hover activation. Right-click tabs to close, duplicate, close tabs to the left or right, or pin/unpin them. Pinned tabs stay on the left, cannot be closed until unpinned, and open other locations in a new tab instead of replacing their pinned location
- **Session restoration and startup location** — Continue the previous session with saved tabs, pane layouts, and view preferences, or start at This PC, Documents, the user profile, or a custom folder; restored Tag tabs display their names immediately
- **New-tab preferences** — Choose the current location, This PC, Documents, the user profile, or a custom folder for new tabs, and optionally open archives in a new tab; middle-click supported sidebar locations to open them in a new tab
- **System tray** — Optionally send the main window to the notification area when closing it; reopen it or exit the app from the tray menu
- **Single or multiple instances** — Choose whether to allow multiple application instances in **Settings > General**. When disabled, launching MTT again restores the existing window, including from the system tray
- **Grid, Details, Column List, and Miller's Columns views** — Switch between thumbnail, detailed table, horizontally scrolling list, and Finder-style hierarchical layouts. Miller's Columns keeps the folder hierarchy visible in side-by-side columns and supports keyboard navigation, multi-selection, rectangle selection, inline renaming, context menus, and drag-and-drop
- **File grouping** — Group items by name, date, type, or size in supported views; reverse the group order, collapse individual sections, and preserve grouping preferences between sessions
- **Configurable shortcuts and view controls** — Customize keyboard and mouse-wheel shortcuts in Settings, including Back, Forward, and Up navigation with support for assigning Backspace. Ctrl+mouse wheel cycles view modes by default, while Ctrl+Shift+mouse wheel adjusts thumbnail size in Grid view
- **File-list preferences** — Independently disable automatic column resizing or hover previews in **Settings > General**; manual column widths support horizontal scrolling. Enable optional alternating row colors in **Settings > Appearance**
- **Date formats** — Choose DD/MM/YYYY, MM/DD/YYYY, or YYYY-MM-DD for displayed dates and Global Search date filters
- **Per-folder view locks** — Preserve the view mode, sorting, and folder position for one folder or for that folder and all of its subfolders; inherited locks identify their source folder
- **Smart address bar** — Direct path input with breadcrumbs
- **Sidebar** — Quick access to drives, libraries, Cloud Drives, and Recycle Bin; sections reclaim space when collapsed, with an adjustable drives area and a separate collapsible Network Locations section. Right-click empty space or section headers to collapse or expand all sections
- **Sidebar renaming** — Rename folders directly in the sidebar tree
- **Network browsing** — Enter a UNC server address to list its shares; browse saved Windows Network Locations from the sidebar or This PC, and launch the Windows Add Network Location wizard
- **Cloud Drives** — Detects Windows Cloud Files sync roots registered with Explorer and shows them in a dedicated sidebar section; tested with OneDrive, Proton Drive and Google Drive
- **Quick Access** — Pin folders via right-click or drag-and-drop; pin/unpin supported folder locations directly from tab context menus; reorder via drag; choose a vertical sidebar section or horizontal bar in Settings > General
- **Archive navigation** — Open supported compressed files like folders and browse their contents directly (`.zip`, `.7z`, `.rar`, `.tar`, `.tar.gz`, `.tgz`, `.tar.bz2`, `.tbz2`, `.tar.xz`, `.txz`, `.tar.zst`, `.tzst`, `.gz`, `.gzip`)

### Tags
- **Persistent color tags** — Assign custom color tags to files and folders from the context menu
- **Dedicated Tag views** — Browse every item assigned to a tag from the resizable Tags section in the sidebar
- **Tag management** — Create, rename, recolor, and delete tags from Settings, with usage counts shown before deletion
- **Global Search filters** — Show any tagged item or narrow search results to specific tags
- **File operation tracking** — Preserve tags across supported renames and moves, and clear them when files are deleted
- **Scalable browsing** — Load large Tag views in pages using cached file metadata while limiting thumbnail work to visible items

### Media Preview
- **Media metadata** — The preview panel extracts video and audio details such as codec, bitrate, channels, sample rate, and music tags
- **Smart thumbnails** — Multi-stage generation: image crate → WIC → Shell API → Media Foundation
- **Selection sizes** — Include folder contents in selection totals, with background calculation and progress updates while folder sizes are resolved; size sorting uses calculated folder sizes as they become available
- **Animated GIF playback** — Animated preview on details panel

### Image, PDF, DOCX, XLSX & Text Viewers
- **Consistent dedicated windows** — Images, PDF documents, DOCX documents, XLSX workbooks, and text files open in lightweight viewer windows with the selected appearance theme, themed toolbars, and native title bar integration
- **Image viewer** — Uses a bounded sliding-window GPU texture cache, hidden-first startup, multi-threaded decoding, zoom controls, and drag-to-pan navigation for zoomed images; copy the displayed image as both a bitmap and file, delete through Windows Shell, crop with a rotation-aware selection and safe save workflow, or switch to full screen with F11. Crops can replace supported raster images after confirmation or be saved as PNG, JPEG, WebP, BMP, or TIFF; animated GIFs export the displayed frame
- **Image conversion** — Convert images in the background, including export to a single-page PDF, with safe destination replacement that preserves existing files on encoding failure and refuses to overwrite files changed after confirmation
- **PDF viewer** — Uses native PDFium rendering with asynchronous document loading, prioritized progressive rendering, virtualized pages and thumbnails, bounded texture caching, and keyboard navigation in the thumbnail sidebar; supports single-page, two-page, and two-page-with-cover layouts
- **DOCX viewer** — Uses BetterOffice's native OOXML parsing, pagination, and raster renderer in a read-only window with zoom and page navigation; supports document search and text selection/copying. Double-click continues to open the Windows default application
- **XLSX viewer** — Uses BetterOffice's native workbook model and rendering in a read-only window with worksheet tabs, zoom, workbook search, and cell-range selection/copying
- **Text viewer** — Opens plain text, source code, logs, and markup files in a focused viewer with go-to-line controls and search that highlights and navigates individual matches

### Video Player
- **Standalone playback** — Opens video and audio files in an mpv-based player with a D3D11 GPU pipeline, without an app-imposed file-size limit
- **Audio visualization** — Displays a real-time waveform while playing audio-only files
- **Playback controls** — Supports 5% volume steps with Up/Down and five-second seeking with the mouse wheel
- **Optical media playback** — Plays DVDs and Blu-rays from an optical drive's context menu
- **Optical media limitations** — DVD menus are not supported; protected DVDs depend on libdvdcss and physical-drive access, while AACS-protected Blu-rays require externally configured libaacs and keys

### Drive Information
- **System overview** — The This PC details panel shows the device name, CPU, installed RAM, GPU, Windows version, storage usage, motherboard, and BIOS information
- **Hardware details** — The details panel can show model, serial number, firmware, interface, standard, supported features, rotation, and current/maximum transfer modes for local drives
- **Health telemetry** — Reads standardized NVMe and ATA/SATA health data such as SMART status, temperature, remaining life, host reads/writes, power cycles, and power-on hours when the device protocol provides those values
- **NVMe link information** — Reports the current and maximum PCIe generation and link width when Windows exposes the corresponding PCI device properties
- **On-demand access** — Hardware queries run through the elevated `mtt-search-service` only when a local drive is shown in the details panel; successful results are cached for five minutes
- **Proactive health status** — Background scheduling preloads drive health when possible and surfaces distinct warning and critical badges in the sidebar, This PC, and drive details
- **Sidebar drive details** — Hover over a drive in the sidebar to see its label, filesystem, capacity, free space, and available hardware metadata
- **Compatibility limits** — Native NVMe and standard ATA/SATA paths are prioritized. SAT-compatible USB bridges are supported on a best-effort basis, while proprietary USB bridges, RAID controllers, multi-disk volumes, and vendor-specific counter formats may not expose all fields

### Disk Usage Analyzer
- **Standalone NTFS analysis** — Launch **Analyze disk usage** from a local NTFS drive's context menu to open an independent analyzer window backed by a full-volume snapshot from the `mtt-search-service` index
- **Visual navigation** — Explore usage through a colorblind-safe treemap, category breakdown, usage chart, and breadcrumb drill-down; open a selected folder directly in the main file manager
- **Metrics and filters** — Compare allocated size, logical size, or file count and narrow results by name or path, extension, category, and minimum or maximum size
- **Largest and efficiency reports** — List the largest files and folders or inspect where logical and allocated sizes differ; Largest Items supports file-count ranking, filter-adjusted values, and multi-column sorting with Shift-click
- **Duplicate detection** — Run an on-demand, cancelable two-stage scan that groups candidates by size and confirms matches with full BLAKE3 hashes; configurable size limits, hardlink awareness, and recoverable-space totals keep the report accurate

### Global Search
- **Instant search** — Query an in-memory index supporting millions of files, with trigram acceleration for eligible volumes and fallback matching for short queries and very large indexes
- **Hybrid volume indexing** — NTFS/ReFS via USN Journal; non-USN volumes via full-tree scan
- **Background service** — Dedicated Windows Service for continuous indexing
- **Spotlight-style overlay** — Activated by Ctrl+Shift+F
- **Paginated results** — Offset/limit pagination with incremental loading
- **Category filters** — Show all results or narrow them to files, folders, images, videos, audio, documents, or applications; Applications includes executables, installers, application shortcuts, and scripts
- **Drive, size, and date filters** — Narrow results to a drive, set minimum and maximum file sizes, or specify a creation-date range
- **Tag filters** — Narrow global search results to any tagged item or to specific tags
- **File interactions** — Select multiple results, use range selection, copy or cut files, rename inline, open the Windows context menu, and drag results to folders or other applications
- **Responsive tagged search** — Tagged results are resolved asynchronously and stale requests are cancelled when the query or filters change
- **Initial indexing feedback** — After an installer-driven index rebuild, a persistent notification reports preparation and live item counts until every indexed volume is ready

> **Disclaimer:** Global Search reads the NTFS/ReFS USN Journal and MFT, and drive health queries access physical storage devices. Because Windows restricts these operations, the installer registers a dedicated Windows Service that runs with administrative rights.

### File Operations
- **Core operations** — Copy, cut, paste, rename, delete
- **Clipboard feedback** — Distinct markers identify copied and cut files and folders, with faded icons for items marked for moving
- **Permanent deletion** — Use Shift+Delete or hold Shift while clicking Delete in the context menu or toolbar to bypass the Recycle Bin
- **Selection controls** — Select all, clear selection, or invert selection from the status bar
- **Folder access assistance** — For access-denied folders, request permanent full control for your Windows account through an administrator-authorized helper; the main file manager stays unelevated. A warning explains that the grant also applies to inheriting files/subfolders and can be used by other programs running under your account
- **Batch rename** — Select 2+ files and press F2 to open the batch rename modal; configure a shared base name, number position (suffix/prefix), separator style (parentheses, underscore, dash, space, or none), and start/step/padding; drag-to-reorder; live preview table with per-row conflict detection
- **Native context menu** — Full Windows Shell context menu integration, including the native **New** submenu when right-clicking an empty folder area
- **Archive creation** — Compress selected files and folders to ZIP or 7Z from the context menu, with byte-level progress, cancellation, conflict-free archive naming, and temporary output cleanup after failures
- **Archive extraction** — Access the Windows Shell **Extract All** action directly from the context menu for supported archive files
- **External drag-and-drop** — Drag files from MTT File Manager to Windows Explorer and other compatible Windows applications
- **Tag assignment** — Add, remove, or switch file/folder tags from the context menu; tags are preserved on supported renames/moves and cleared when files are deleted
- **Recycle Bin** — Browse, restore, and permanently delete; drag selected items to the Recycle Bin target in Quick Access to send them there
- **Cloud Files support** — Sync status badges for cloud-only, locally available, syncing, and pinned files; supports Windows Cloud Files actions such as "Always keep on this device" and "Free up space"
- **Google Drive integration** — Read Google Drive sync status through its Windows Shell integration and resolve folder shortcuts for navigation
- **ISO mounting** — Mount ISO files as virtual drives, detect images mounted before the app starts, and eject them from the sidebar

### Automatic File Organizer
- **Persistent rules** — Automatically move files from a source folder to a destination based on file extensions
- **Extension presets** — Quickly configure rules for documents, images, videos, audio, archives, and executables, or enter custom extensions
- **Conflict policies** — Resolve destination conflicts manually, skip them, automatically rename the source, or redirect it to a designated conflict folder; existing files are never overwritten
- **Pending conflict management** — Keep collisions pending, safely rename the source, rename the occupied destination after confirmation, or cancel the conflict with file identity checks before every change
- **Operation history** — Track organizer moves and their status with configurable retention, retry eligible operations using the current rule, and safely undo completed moves while file identities remain valid
- **Rule controls and recovery** — Pause or resume rules, inspect live monitoring status, create missing source or destination folders with confirmation, and recover automatically from watcher failures and event queue overflow
- **Safe folder monitoring** — Existing and newly created matching files are processed only after remaining stable for two seconds; source, destination, and conflict folders are monitored so resolved collisions can be reprocessed automatically
- **Protected moves and rule validation** — Cyclic rules are rejected, source files are checked before moving, and cross-drive copies are verified before their sources are removed
- **Preview and notifications** — Preview how many files match a disabled rule and receive batched details about completed moves, policy-based skips, pending conflicts, and failures

### Performance & Cache
- **Multi-level cache** — Memory, disk (SQLite), and GPU textures
- **Tag view cache** — Persistent metadata cache and paged loading make large Tag views appear quickly without preloading thumbnails or increasing GPU texture cache limits
- **Async workers** — Background processing keeps UI responsive
- **UI virtualization** — Efficient rendering of large directories
- **Bounded background work** — Folder loading, thumbnail generation, and search rescans use concurrency limits to avoid resource spikes and improve shutdown behavior
- **Idle memory trimming** — Trims the Windows process working set after idle periods when it exceeds a configured threshold
- **Per-folder monitoring** — Default `notify` crate watcher with opt-in drive-wide `ReadDirectoryChangesW`; event bursts are coalesced into debounced reloads

## Optional Windows Integration

These options are available in **Settings > General**. They modify the current user's Registry settings without requiring administrator privileges or UAC confirmation.

> **Warning:** Both options are experimental. Some antivirus software may detect this Registry-modifying behavior as malware and report a false positive.

### Set MTT as the default file manager

Use MTT as the default handler for filesystem folders and drives through Windows' existing **Open** action, without adding duplicate folder context-menu entries. Obsolete MTT entries belonging to the current installation are cleaned up. Virtual folders such as Control Panel retain their native Windows handlers.

### Open MTT with Win+E

This separate option redirects Win+E and Explorer launch requests to MTT.

Pressing Win+E and running explorer.exe will open MTT instead of Windows File Explorer. In situations where specific execution of explorer.exe is required, this will not be possible. This option modifies the current user's Registry settings.

## Graphics Backend

The app supports three rendering backend choices, selectable in **Settings > Graphics > GPU Backend** (requires app restart):

### Wgpu — DirectX 12 (Default)
- Default backend for the main window on Windows
- Uses `wgpu` DX12 with a DirectComposition visual for presentation
- Avoids transient black frames when the borderless window is minimized
- If DX12 initialization fails, startup automatically retries Vulkan and then Glow + OpenGL

### Wgpu — Vulkan
- Optional Vulkan backend for the main window
- Keeps the Vulkan-specific thumbnail and grid performance tuning
- Does not use the DX12 DirectComposition presentation path

### Glow — OpenGL (Fallback)
- Fallback when DirectX 12 or Vulkan is unavailable or unstable on your system.
- Uses eframe's `Glow` renderer directly instead of `wgpu`'s OpenGL backend
- OpenGL texture uploads can be synchronous on the CPU thread, leading to some stutter.

Dedicated image, PDF, DOCX, XLSX, and text viewer windows start with Glow independently of the main window's backend setting. If OpenGL initialization fails before the viewer is created, they retry with Wgpu (DirectX 12 on Windows).

## Prerequisites

- **Windows 10 or newer, 64-bit** — The installer targets x64-compatible Windows systems.
- **Rust 1.85 or newer for source builds** — The pinned BetterOffice DOCX and XLSX crates use the Rust 2024 edition; the application package itself remains edition 2021.
- **Microsoft Visual C++ Redistributable 2015-2022 (x64)** — Required by the native runtime dependencies. The official Microsoft installer is available at: https://aka.ms/vs/17/release/vc_redist.x64.exe
- **Administrator permission during installation** — Required to install and start the Global Search and drive telemetry Windows Service (`mtt-search-service.exe`).
- **Video codecs for extended thumbnail support** — Optional, but recommended for formats not supported by Windows out of the box. See [Video Thumbnail Codecs](#video-thumbnail-codecs).

The main file manager does not need to run as administrator for normal file browsing and file operations. Elevated service access is isolated in `mtt-search-service.exe`, which indexes NTFS/ReFS volumes using low-level filesystem data and performs on-demand health queries against physical storage devices. The installer registers this dedicated Windows Service with the required privileges instead of requiring the whole application to run elevated.

## Technologies

| Category | Technology | Version | Purpose |
|----------|-----------|---------|---------|
| **Language** | Rust | Edition 2021 | Performance and safety |
| **GUI** | eframe/egui | 0.35 | Modern immediate-mode GUI (features: `persistence`, `wgpu`, `glow`) |
| **GPU Backend (Default)** | wgpu DirectX 12 via eframe | 29.x | DirectComposition presentation; requires app restart |
| **GPU Backend (Alternative)** | wgpu Vulkan via eframe | 29.x | Optional Vulkan renderer; requires app restart |
| **GPU Backend (Fallback)** | Glow OpenGL via eframe | via eframe | OpenGL fallback; requires app restart |
| **Windows API** | windows-rs | 0.62 | Native Windows integration |
| **Video** | libmpv2 | 5.0.3 | High-performance video playback |
| **PDF** | pdfium (pdfium-render) | 0.8.37 | Native PDF rendering (requires pdfium.dll) |
| **DOCX** | BetterOffice DOCX parse/layout/raster | 0.3.0 | Native OOXML pagination and read-only page rendering |
| **XLSX** | BetterOffice XLSX model/parse/render/raster | 0.3.0 | Native workbook reading and read-only sheet rendering |
| **Database** | SQLite (rusqlite) | 0.32 | Reliable persistence |
| **Images** | image crate | 0.25 | Image processing |
| **Archives** | zip + sevenz-rust/sevenz-rust2 + tar + flate2/bzip2/xz2/zstd | 2 / 0.6/0.20 / 0.4 / 1 / 0.5 / 0.1 / 0.13 | Native archive browsing plus ZIP/7Z creation |
| **RAR** | unrar | 0.5 | Native RAR handling via the upstream UnRAR source |
| **Parallelism** | rayon | 1.10 | Parallel processing |
| **IPC** | Named Pipes + bincode | 1.3 | App ↔ search service communication |
| **Service** | windows-service | 0.7 | Background indexing and on-demand drive telemetry service |
| **i18n** | rust-i18n | 3 | Multi-language support (en, pt-BR, zh-CN, ru, el) |

### Runtime Dependencies
- **libmpv-2.dll** — Required for video playback
- **pdfium.dll** — Required for PDF viewer
- **Video codecs** — Required for video thumbnail extraction (see [Video Thumbnail Codecs](#video-thumbnail-codecs) below)

## Privacy

### No tracking or automatic data uploads

MTT does not collect user or machine information for analytics, advertising, or profiling, and does not automatically upload diagnostic reports. File browsing, search indexing, previews, and hardware information are processed on your computer; file metadata and device information used by these features are not sent to the project or the update service.

### Internet access and updates

MTT's built-in Internet requests are used exclusively to check for application updates, retrieve release notes, and download the installer needed to perform an update. These HTTPS requests go to the official GitHub repository and GitHub's download infrastructure, without account credentials or uploads of files, search terms, diagnostics, or machine identifiers. Installation then runs locally through Windows setup.

Update requests identify the application and its version in the User-Agent header. Like any Internet connection, they expose the connection's public IP address to the service provider; this is not an anonymous connection. GitHub handles these requests under its own [privacy statement](https://docs.github.com/en/site-policy/privacy-policies/github-general-privacy-statement).

Startup checks are enabled by default and can be disabled in **Settings > About**. Automatic startup installation is a separate option and is disabled by default; manual checks and installation remain available.

This update-only scope does not mean that user-requested actions are offline: opening web links uses your browser, accessing network locations contacts the selected server, and accessing cloud-backed files may cause the installed cloud provider to download or synchronize them. Those services have their own privacy policies.

### Optional local diagnostics

- Enable **Settings > Diagnostics** only when needed for troubleshooting. It records technical information about the application's operation, such as errors, timings, counters, and resource usage, along with the application version, operating-system family, and architecture.
- Before writing diagnostic text, the logger filters recognized file and folder names, filesystem paths, network server/share names, drive labels, search terms, and other sensitive identifiers. Filtering reduces exposure but is not a guarantee that every possible identifier will be recognized; inspect the plain-text file before sharing it.
- The diagnostic file stays on your computer. It is never uploaded automatically, including during updates. You decide whether, when, and with whom to share it.
- Diagnostic mode auto-disables after 24 hours and limits the file to approximately 10 MiB while retaining recent filtered events.
- These are technical privacy measures for data minimization and safer troubleshooting, not a standalone legal certification of LGPD or any other regulatory compliance.

## Temporary GPU Recovery Tests

- Glow preserves its GPU texture handles after prolonged background or minimize recovery by default.
- Set `MTT_OPENGL_FLUSH_TEXTURES=1` to re-enable the OpenGL texture flush for comparison.
- Set `MTT_DISABLE_WS_TRIM=1` to disable process working-set trimming for comparison.
- Set `MTT_TRACE_FRAME_GAPS=1` to log visible frame gaps even when thumbnail work is idle.
- These variables are intended for local performance tests and are not required for normal use.

## Video Thumbnail Codecs

The thumbnail pipeline uses 3 Windows APIs for video files: **Shell API** (Stage 3), **IThumbnailCache** (Stage 4), and **Media Foundation** (Stage 5). All three require video codecs to be registered on the system.

### What works out of the box (Windows 10/11)
- **MP4 (H.264/AVC)**, **WMV**, **AVI** — native Windows codecs

### What requires installation

| Format | Without codecs | With K-Lite Codec Pack |
|--------|---------------|------------------------|
| MP4 H.264 | ✅ Works | ✅ Works |
| MP4 HEVC/H.265 | ❌ Fails | ✅ Works |
| MKV (any codec) | ❌ Fails | ✅ Works |
| WEBM VP9/AV1 | ❌ Fails | ✅ Works |
| FLV | ❌ Fails | ✅ Works |

### Recommended: K-Lite Codec Pack

**[Download K-Lite Codec Pack (Standard)](https://codecguide.com/download_kl.htm)** — includes LAV Filters which register:
- **Thumbnail handlers** for Windows Shell (enables Stages 3 and 4)
- **Media Foundation decoders** (enables Stage 5)
- Support for **HEVC/H.265**, **VP9**, **AV1**, **MKV**, **WEBM**, **FLV**, and more

> **Note**: Without the appropriate codecs installed, all video thumbnail stages will fail silently and the file will display a generic icon instead.

## Credits

This project includes and builds upon work from the following projects:

- [ModernH](https://github.com/HarkeshBhatia/ModernH), by Harkesh Bhatia. Our OSC file originated from this project and is used here with small modifications.
- [RTX HDR / RTX VSR toggle gist](https://gist.github.com/anthonybaldwin/1e49b28b49babf64f159cb793c506333), by anthonybaldwin. This gist served as an early development reference while experimenting with RTX HDR / RTX VSR behavior in mpv; the current repository implementation has since been reworked independently.
- [BetterOffice](https://github.com/openooxml/betteroffice), by Elia Hilse and The OpenOOXML Project. MTT uses its Apache-2.0 DOCX and XLSX engines for the read-only document and workbook viewers; the upstream attribution notice is included in `third_party_licenses/` and public installers.

### Translations

- **Chinese (Simplified)** — contributed by [@cdtsuipo](https://github.com/cdtsuipo)
- **Russian** — contributed by [@Nekiplay](https://github.com/Nekiplay)

## License

Except where otherwise noted, the original code and documentation authored for this repository are licensed under the **Apache License, Version 2.0**. See the top-level `LICENSE` and `NOTICE` files.

Apache-2.0 was chosen because it fits the current Rust stack well and gives a clear attribution baseline: anyone redistributing the Apache-licensed portions of this project must preserve the copyright notice, the license text, and any applicable `NOTICE` entries. In practice, this lets the project require retention of legal attribution, but it does **not** force public branding, UI credits, or endorsement for every downstream project.

This repository also contains or redistributes third-party components that remain under their own licenses and are not relicensed under Apache-2.0. Key examples include:

- `mpv_ui/portable_config/scripts/modernH.lua` and `mpv_ui/portable_config/script-opts/osc.conf`, derived from ModernH and kept under LGPL-2.1.
- `mpv_ui/portable_config/scripts/autoload.lua`, copied from upstream mpv tooling and governed by upstream mpv licensing.
- `target\release\libmpv-2.dll`, whose redistribution terms depend on the exact binary build shipped.
- `target\release\pdfium.dll`, which carries upstream PDFium licensing and notice obligations independent of the Rust bindings.
- `mpv_ui/portable_config/fonts/Material-Design-Iconic-Font.ttf`, which has its own upstream asset license.
- `unrar`, whose Rust wrapper is permissive but whose embedded UnRAR sources retain the upstream UnRAR license.
- BetterOffice's DOCX and XLSX engines, under Apache-2.0, and the shared Yrs
  0.27.3 dependency, under MIT. The upstream attribution notice and applicable
  license texts are included with the installer; the vendored Yrs copy contains
  a documented stable-Rust compatibility change. Raster/ZIP dependencies also
  retain their BSD-3-Clause, MIT, and Zlib notices in `third_party_licenses/`.

The official Windows installer is therefore a multi-license distribution, not
an Apache-only artifact. Public redistribution is intended to be allowed when
the installer keeps the bundled notices/license texts, the matching source code
or source locations remain available, and third-party components are not
described as being relicensed under Apache-2.0.

For practical release guidance, see `THIRD_PARTY_NOTICES.md` and the
`third_party_licenses/` bundle. Public installers include that directory, which
contains full license texts, attribution notes, source availability notes, and
release-sensitive binary provenance.
