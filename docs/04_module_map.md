# Module Map — MTT File Manager

## Directory Structure

```
src/
├── main.rs                          # Entry point, viewport config, CLI dispatch
├── lib.rs                           # Crate root, i18n macro, module declarations
├── embedded_assets.rs               # Embedded resources for portable executable
├── viewer_runtime.rs                # Shared runtime config for image/PDF/text viewers
│
├── app/                             # Application state & initialization
│   ├── mod.rs                       # Module declarations
│   ├── state/                       # ImageViewerApp — main app state (split module)
│   │   ├── mod.rs                   # ImageViewerApp struct definition & core state
│   │   └── helpers.rs               # State helper methods & utilities
│   ├── batch_rename.rs              # Batch rename domain logic: BatchRenameState, name generation, conflict detection, drag-reorder state
│   ├── cache_state.rs               # Cache state management
│   ├── drive_state.rs               # Drive information state
│   ├── dual_panel.rs                # Dual panel state types: ActivePanel enum, PanelSnapshot struct (captures/restores per-panel browsing state via from_app/apply_to/swap_with_app)
│   ├── file_operation_state.rs      # File operation tracking
│   ├── folder_size_state.rs         # Folder size caches, batch invalidation, stale-result guards
│   ├── global_search_state.rs       # Global search session state
│   ├── layout_state.rs              # Layout preferences
│   ├── live_file_size.rs            # Live file size monitoring state
│   ├── navigation_state.rs          # Navigation state, history & ThemeMode enum
│   ├── shortcuts.rs                 # Keyboard shortcuts definitions
│   ├── ui_state.rs                  # UI preferences state
│   ├── init.rs                      # ImageViewerApp::new() initialization
│   ├── init_bootstrap.rs            # Bootstrap sequence
│   ├── init_post_startup.rs         # Post-startup initialization
│   ├── init_preferences.rs          # Preference loading from app_state.db
│   ├── init_state_builders.rs       # State builder utilities
│   ├── init_workers/                # Worker initialization
│   │   ├── mod.rs
│   │   ├── background_jobs.rs       # Background task schedulers
│   │   ├── consistency_probe_worker.rs  # FS consistency checking
│   │   ├── filesystem_workers.rs    # Folder size workers (NTFS IPC + FS fallback), folder preview, cache invalidation
│   │   ├── pipeline_workers.rs      # File ops, global search, prefetch workers
│   │   └── visual_workers.rs        # Font loading, cover, icon, metadata workers
│   └── operations/                  # Business logic operations
│       ├── mod.rs
│       ├── clipboard_ops.rs         # Copy/paste operations
│       ├── context_menu.rs          # Right-click menu handling
│       ├── drag_drop/               # Drag-and-drop support (split module)
│       │   ├── mod.rs               # Drag-and-drop orchestration
│       │   ├── validation.rs        # Drop target validation
│       │   └── rendering.rs         # Drag ghost & visual feedback rendering
│       ├── dual_panel_ops.rs        # Dual panel enable/disable/toggle, switch active panel, with_inactive_panel closure helper
│       ├── file_ops.rs              # OS-level file operations
│       ├── folder_loading/          # Async folder loading
│       ├── folder_lock_ops.rs       # Per-folder view preferences
│       ├── icons.rs                 # Icon extraction
│       ├── message_handler/         # Window message handling
│       ├── metadata.rs              # File metadata extraction
│       ├── navigation/              # Navigation operations
│       │   ├── keyboard.rs          # Keyboard navigation
│       │   └── selection.rs         # Selection navigation
│       ├── pinned_folder_ops.rs     # Quick Access pinned folders
│       ├── preferences.rs           # User preferences
│       ├── recycle_bin_ops.rs       # Recycle Bin operations
│       ├── selection.rs             # Item selection logic
│       ├── shutdown.rs              # Graceful shutdown
│       ├── tabs.rs                  # Tab operations
│       ├── thumbnails.rs            # Thumbnail generation
│       ├── trait_impls.rs           # Trait implementations
│       ├── ui_rendering/            # Grid/list bridge rendering
│       ├── view_setup.rs            # View initialization
│       ├── watcher.rs               # Watcher integration
│       └── window.rs                # Window management
│
├── application/                     # Application services
│   ├── mod.rs
│   ├── clipboard.rs                 # Clipboard manager
│   ├── context_menu.rs              # Context menu logic
│   ├── file_operations.rs           # File copy/move/delete
│   ├── navigation.rs                # Navigation history
│   ├── notification.rs              # Toast notifications
│   ├── renaming.rs                  # File rename logic
│   ├── sorting.rs                   # Sorting facade
│   ├── sorting/                     # Sorting implementation
│   │   ├── sort_impl.rs             # Sort algorithms
│   │   └── filtering.rs             # Filter logic
│   └── watcher.rs                   # Filesystem watcher integration
│
├── domain/                          # Core data models
│   ├── mod.rs
│   ├── errors.rs                    # AppError enum, helper macros & traits
│   ├── file_entry.rs                # FileEntry, DriveInfo, SortMode, ViewMode, etc.
│   ├── folder_lock.rs               # FolderLock struct
│   ├── pinned_folder.rs             # PinnedFolder struct
│   ├── special_paths.rs             # System paths (Computer, Recycle Bin)
│   └── thumbnail.rs                 # ThumbnailData struct
│
├── infrastructure/                  # System integration
│   ├── mod.rs
│   ├── adaptive_batch.rs            # Adaptive batch sizing
│   ├── app_state_db/                # App state SQLite store
│   │   ├── mod.rs                   # AppStateDb entry point
│   │   ├── cleanup.rs               # State cleanup helpers
│   │   ├── folder_covers.rs         # Folder cover persistence
│   │   ├── folder_locks.rs          # Folder lock persistence
│   │   ├── gc.rs                    # State garbage collection
│   │   ├── pinned_folders.rs        # Pinned folder persistence
│   │   └── preferences.rs           # Preference persistence
│   ├── archive_extract.rs           # Native archive extraction fallback (ZIP, 7z, RAR, TAR)
│   ├── db_utils.rs                  # Shared SQLite ACL/PRAGMA/fallback helpers
│   ├── directory_cache.rs           # In-memory directory cache
│   ├── directory_dirty_registry.rs  # Directory dirty state tracking
│   ├── directory_index.rs           # Persisted directory metadata cache (directory_cache.db)
│   ├── disk_cache.rs                # Thumbnail / preview / shell icon SQLite entry point
│   ├── disk_cache/                  # Thumbnail cache submodules
│   │   ├── cleanup.rs               # Cache cleanup
│   │   ├── folder_previews.rs       # Folder preview cache
│   │   ├── gc.rs                    # Garbage collection
│   │   ├── shell_icons.rs           # Shell icon cache
│   │   └── thumbnails_repo.rs       # Thumbnail repository
│   ├── drive_watcher.rs             # Drive-wide watcher (ReadDirectoryChangesW)
│   ├── drive_watcher/               # Watcher submodules
│   │   ├── buffer_parser.rs         # Change buffer parsing
│   │   └── thread_loop.rs           # Watcher thread loop
│   ├── folder_compose.rs            # Folder cover composition (3-layer PNG)
│   ├── global_search.rs             # Named Pipe client for search IPC
│   ├── icon_disk_cache.rs           # Icon disk cache
│   ├── io_priority.rs               # I/O priority entry point
│   ├── io_priority/                 # I/O priority submodules
│   │   ├── detection.rs             # Storage type detection
│   │   ├── grouped_queue.rs         # Priority queue grouping
│   │   └── threading.rs             # Thread priority management
│   ├── media/                       # Media infrastructure
│   │   ├── mod.rs
│   │   └── hardware_acceleration.rs # HW acceleration detection
│   ├── ntfs_reader.rs               # Raw NTFS directory reading
│   ├── onedrive/                    # OneDrive integration
│   │   ├── mod.rs
│   │   ├── attributes.rs            # OneDrive file attributes
│   │   ├── directory_enum.rs        # OneDrive directory enumeration
│   │   ├── path_detection.rs        # OneDrive path detection
│   │   ├── pin_state.rs             # OneDrive pin commands
│   │   └── timeout_ops.rs           # Timeout operations
│   ├── security.rs                  # Security validation entry point
│   ├── security/                    # Security submodules
│   │   ├── components.rs            # Security components
│   │   ├── drive.rs                 # Drive security
│   │   ├── shell_namespace.rs       # Shell namespace validation
│   │   ├── symlink.rs               # Symlink validation
│   │   └── unc.rs                   # UNC path validation
│   ├── shell_menu_worker.rs         # Shell context menu extraction
│   ├── threading.rs                 # Named thread spawning utilities
│   ├── user_session_search/         # User session search index (split module)
│   │   ├── mod.rs                   # Search index orchestration
│   │   ├── db.rs                    # SQLite persistence for search index
│   │   ├── discovery.rs             # Volume & path discovery
│   │   └── scanner.rs               # Directory scanning & indexing
│   ├── virtual_drive_config.rs      # Virtual drive configuration
│   ├── windows_clipboard.rs         # Windows clipboard (CF_HDROP)
│   ├── windows/                     # Windows-specific APIs
│   │   ├── mod.rs
│   │   ├── bitmap_conversion.rs     # Bitmap conversion
│   │   ├── codec_registry.rs        # Codec name cache
│   │   ├── codec_registry/          # Codec registry submodules
│   │   ├── device_change.rs         # Device change monitoring
│   │   ├── drives.rs                # Drive enumeration
│   │   ├── file_flags.rs            # File attribute flags
│   │   ├── file_system.rs           # Filesystem operations
│   │   ├── file_type.rs             # File type detection
│   │   ├── folder_size.rs           # Folder size calculation
│   │   ├── formatting.rs            # Number/string formatting
│   │   ├── hdd_directory_reader.rs  # Optimized HDD directory reader
│   │   ├── icons.rs                 # Icon extraction
│   │   ├── icons/                   # Icon submodules
│   │   ├── iso_mount.rs             # ISO file mounting
│   │   ├── media_foundation.rs      # Media Foundation integration
│   │   ├── metadata/                # Metadata extraction submodules
│   │   ├── native_menu.rs           # Native Windows context menu
│   │   ├── recycle_bin.rs           # Recycle Bin operations
│   │   ├── recycle_bin/             # Recycle Bin submodules
│   │   ├── shell_folder.rs          # Shell special folders
│   │   ├── shell_operations.rs      # Shell file operations
│   │   ├── shell_operations/        # Shell operations submodules
│   │   ├── system_info.rs           # System information
│   │   ├── window_corners.rs        # Window corner styling & dark title bar (DWM)
│   │   └── window_subclass.rs       # Window subclassing
│
├── ui/                              # User interface
│   ├── mod.rs
│   ├── app_impl.rs                  # eframe::App implementation
│   ├── cache.rs                     # Texture/icon cache manager
│   ├── theme.rs                     # UI theming (color constants, dark-mode-aware helpers)
│   ├── widgets.rs                   # Custom egui widgets
│   ├── svg_icons.rs                 # SVG icon renderer (duotone dark-mode support)
│   ├── toolbar.rs                   # Top toolbar
│   ├── sidebar.rs                   # Side panel
│   ├── sidebar_tree.rs              # Tree sidebar for folder navigation
│   ├── navigation.rs                # Navigation UI
│   ├── status_bar.rs                # Bottom status bar
│   ├── context_menu.rs              # Context menu renderer
│   ├── app/                         # App lifecycle submodules
│   │   ├── mod.rs
│   │   ├── input.rs                 # Input handler
│   │   ├── lifecycle.rs             # App lifecycle
│   │   ├── layers.rs                # UI layers
│   │   ├── layers/                  # Layer submodules
│   │   ├── menu_handler.rs          # Menu handler
│   │   ├── notifications.rs         # Notification rendering
│   │   └── panels/                  # Panel layout (split module)
│   │       ├── mod.rs               # render_panels entry, sidebar, resize handles
│   │       └── content.rs           # Preview panel & central panel content rendering; dual panel split layout (left/right rects, path headers, focus switching)
│   ├── tab_bar/                     # Tab system
│   ├── preview_panel/               # Preview panel with video support
│   ├── icon_loader.rs               # Icon loading
│   ├── icon_loader/                 # Icon loader submodules
│   ├── global_search_overlay.rs     # Global search overlay entry point
│   ├── global_search_overlay/       # Search overlay submodules
│   │   ├── results_panel.rs         # Results panel layout
│   │   ├── actions.rs               # Search action handlers
│   │   ├── result_row.rs            # Individual result row rendering
│   │   ├── scrollbar.rs             # Custom scrollbar widget
│   │   └── filters.rs               # Search filter logic
│   ├── components/                  # Reusable UI components
│   │   ├── mod.rs
│   │   ├── appearance_settings.rs   # Theme (dark/light) settings component
│   │   ├── batch_rename_modal.rs    # Batch rename modal: controls, drag-reorder list, live preview table, conflict banner
│   │   ├── gif_manager.rs           # GIF playback manager
│   │   ├── item_slot/               # Item slot rendering (drive/folder/file)
│   │   ├── language_settings.rs     # Language settings component
│   │   ├── media_preview.rs         # Media preview component
│   │   ├── mpv/                     # mpv integration
│   │   ├── mpv_preview/             # mpv preview bridge
│   │   ├── video_controls_state.rs  # Video controls state
│   │   └── virtual_drive_settings.rs # Virtual drive settings
│   └── views/                       # File list views
│       ├── mod.rs
│       ├── common.rs                # Shared view utilities
│       ├── computer_view.rs         # "This PC" view
│       ├── grid_view/               # Grid view
│       └── list_view/               # List view
│           ├── mod.rs               # Module declarations, ListViewContext, ColumnWidths
│           ├── header.rs            # Column header rendering
│           ├── helpers.rs           # Shared helpers (file type strings, status badges)
│           ├── item_renderer.rs     # render_list_item entry, selection, rename, column data
│           ├── item_renderer_details.rs  # Tooltip rendering, icon rendering for list items
│           └── virtualization.rs    # Virtual scrolling / row recycling
│
├── tabs/                            # Tab management
│   └── mod.rs                       # TabState struct, per-tab history/sort/view/selection
│
├── workers/                         # Background workers
│   ├── mod.rs
│   ├── file_operation_worker.rs     # Async file operations
│   ├── file_operation_worker/       # File op worker submodules
│   ├── folder_preview_worker.rs     # Folder cover generation
│   ├── global_search_worker.rs      # Global search IPC worker
│   ├── idle_warmup.rs               # Idle-time cache warmup
│   ├── prefetch_worker.rs           # Directory prefetching
│   └── thumbnail/                   # Thumbnail system
│       ├── mod.rs
│       ├── queue.rs                 # Thumbnail priority queue
│       ├── types.rs                 # Thumbnail types
│       ├── worker.rs                # Thumbnail worker entry
│       ├── worker/                  # Worker submodules
│       ├── processing/              # Post-processing
│       └── extraction/              # Multi-stage extraction
│           ├── mod.rs
│           ├── stage0_embedded_exif_thumbnail.rs
│           ├── stage1_image_crate.rs
│           ├── stage2_wic.rs
│           ├── stage3_shell_api.rs
│           ├── stage4_force_extract.rs
│           └── stage5_media_foundation.rs
│
├── image_viewer/                    # Dedicated image viewer (separate process)
│   ├── mod.rs                       # Process spawn & standalone runner
│   ├── app/                         # DedicatedImageViewerApp (split module)
│   │   ├── mod.rs                   # App struct, construction, navigation, cache/prefetch, shortcuts, eframe::App impl
│   │   ├── filmstrip.rs             # FilmstripState, filmstrip thumbnail strip rendering
│   │   ├── gif_export.rs            # GIF animation playback, image export/conversion
│   │   └── rendering.rs             # Top bar, bottom bar, center viewport rendering
│   ├── cache.rs                     # WindowCache + PrefetchEngine
│   ├── indexer.rs                   # Image sequence builder
│   ├── ipc.rs                       # Inter-process communication
│   ├── loader.rs                    # Image decoding (mmap, EXIF, WIC)
│   └── metrics.rs                   # Performance metrics
│
├── text_viewer/                     # Native text viewer (separate process)
│   ├── mod.rs                       # Process spawn, path validation, standalone runner
│   └── viewer_app.rs                # Text viewer state & rendering
│
├── video_player/                    # Standalone video player (separate process)
│   └── mod.rs                       # mpv-based player with D3D11 pipeline
│
└── pdf_viewer/                      # Native PDF viewer (separate process)
    ├── mod.rs                       # Process spawn, path validation
    ├── viewer_app.rs                # PDF viewer state & rendering
    ├── renderer.rs                  # Page rendering
    ├── render_worker.rs             # Async render worker
    ├── selection.rs                 # Text selection support
    └── toolbar.rs                   # PDF controls
```

## Workspace Crates

```
crates/
├── mtt-search-protocol/             # Shared IPC types
│   └── src/
│       └── lib.rs                    # SearchRequest/SearchResponse enums, SearchResultItem, IndexStatusInfo, VolumeStatus, bincode serialization
│
└── mtt-search-service/              # Windows Service for file indexing
    └── src/
        ├── main.rs                   # Entry point + SCM integration
        ├── usn_journal.rs            # Volume discovery + USN API (NTFS/ReFS)
        ├── fs_walker.rs              # Full-tree scanner for non-USN volumes
        ├── file_index.rs             # In-memory HashMap index
        ├── path_resolver.rs          # Path reconstruction via FRN chain
        ├── index_db/                 # SQLite persistence (split module)
        │   ├── mod.rs               # DB initialization, schema, dirty-shutdown handling, shared data dir, ACL hardening
        │   ├── binary.rs            # Per-volume binary snapshot save/load with CRC
        │   ├── integrity.rs         # Snapshot integrity verification
        │   └── sync.rs              # Record persistence, incremental sync, legacy FTS maintenance helpers
        ├── ipc_server/               # Named Pipe server (split module)
        │   ├── mod.rs               # Server loop, client accept, wait_for_client
        │   ├── pipe_io.rs           # Pipe creation (DACL/ACL security), read/write I/O
        │   └── handler.rs           # Request dispatch, status response, tests
        ├── ipc_authorization.rs      # IPC authorization
        ├── security_policy.rs        # Security policy
        ├── service_control.rs        # Service install/uninstall
        ├── name_arena.rs             # String arena for name storage
        └── volume_indexers/           # Per-volume indexer management (split module)
            ├── mod.rs               # Indexer orchestration & shared types
            ├── non_usn.rs           # Full-tree scanner indexer (non-NTFS/ReFS)
            └── usn.rs               # USN journal indexer (NTFS/ReFS)
```

## Configuration Files

```
Cargo.toml                           # Workspace root + mtt-file-manager dependencies
build.rs                             # Windows icon + DPI manifest embedding + pdfium.dll staging + SHA-256 verification
app.manifest                         # DPI awareness and Windows compatibility manifest
virtual_drive_config.json            # Drive letter → storage type overrides
locales/
├── en.yml                           # English translations
└── pt-BR.yml                        # Brazilian Portuguese translations
assets/icons/gen/                    # Generated icon assets
mpv_ui/portable_config/              # mpv configuration
├── cache/
├── mpv.conf
├── script-opts/
└── scripts/
benches/
├── image_viewer_decode.rs            # Image viewer decode benchmark
└── shell_ops_blocking.rs            # Shell operations benchmark
```

## Data Flow

```
User Input (keyboard/mouse)
    ↓
UI Layer (src/ui/)
    ↓
App State (src/app/state/) ←→ Application Services (src/application/)
    ↓                                    ↓
Operations (src/app/operations/)     Domain Models (src/domain/)
    ↓
Workers (src/workers/) ←→ Infrastructure (src/infrastructure/)
    ↓                            ↓
Background Results ──→ UI Update Loop (channels)
```

## Module Rules

1. **UI** modules may read from `app/state` but must never call infrastructure directly
2. **Application** modules orchestrate between domain and infrastructure
3. **Domain** modules have zero dependencies on infrastructure or UI
4. **Infrastructure** modules depend only on domain types and external crates
5. **Workers** are spawned during init and communicate exclusively via channels
