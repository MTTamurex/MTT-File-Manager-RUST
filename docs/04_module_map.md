# Module Map — MTT File Manager

## Directory Structure

```
src/
├── main.rs                          # Entry point, viewport config, CLI dispatch
├── lib.rs                           # Crate root, i18n macro, module declarations
├── embedded_assets.rs               # Embedded resources for portable executable
│
├── app/                             # Application state & initialization
│   ├── mod.rs                       # Module declarations
│   ├── state.rs                     # ImageViewerApp — main app state struct
│   ├── cache_state.rs               # Cache state management
│   ├── drive_state.rs               # Drive information state
│   ├── file_operation_state.rs      # File operation tracking
│   ├── folder_size_state.rs         # Folder size computation state
│   ├── global_search_state.rs       # Global search session state
│   ├── layout_state.rs              # Layout preferences
│   ├── navigation_state.rs          # Navigation state & history
│   ├── ui_state.rs                  # UI preferences state
│   ├── init.rs                      # ImageViewerApp::new() initialization
│   ├── init_bootstrap.rs            # Bootstrap sequence
│   ├── init_post_startup.rs         # Post-startup initialization
│   ├── init_preferences.rs          # Preference loading from SQLite
│   ├── init_state_builders.rs       # State builder utilities
│   ├── init_workers/                # Worker initialization
│   │   ├── mod.rs
│   │   ├── background_jobs.rs       # Background task schedulers
│   │   ├── consistency_probe_worker.rs  # FS consistency checking
│   │   ├── filesystem_workers.rs    # Folder size, folder preview, cache invalidation
│   │   ├── pipeline_workers.rs      # File ops, global search, prefetch workers
│   │   └── visual_workers.rs        # Font loading, cover, icon, metadata workers
│   └── operations/                  # Business logic operations
│       ├── mod.rs
│       ├── clipboard_ops.rs         # Copy/paste operations
│       ├── context_menu.rs          # Right-click menu handling
│       ├── drag_drop.rs             # Drag-and-drop support
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
│   ├── directory_cache.rs           # In-memory directory cache
│   ├── directory_index.rs           # Directory index for fast lookup
│   ├── disk_cache.rs                # SQLite disk cache entry point
│   ├── disk_cache/                  # Disk cache submodules
│   │   ├── cleanup.rs               # Cache cleanup
│   │   ├── folder_covers.rs         # Folder cover cache
│   │   ├── folder_locks.rs          # Folder lock persistence
│   │   ├── folder_previews.rs       # Folder preview cache
│   │   ├── gc.rs                    # Garbage collection
│   │   ├── pinned_folders.rs        # Pinned folder persistence
│   │   ├── preferences.rs           # Preference persistence
│   │   ├── shell_icons.rs           # Icon cache
│   │   └── thumbnails_repo.rs       # Thumbnail repository
│   ├── drive_watcher.rs             # Drive-wide watcher (ReadDirectoryChangesW)
│   ├── drive_watcher/               # Watcher submodules
│   │   ├── buffer_parser.rs         # Change buffer parsing
│   │   └── thread_loop.rs           # Watcher thread loop
│   ├── drive_watcher_integration.rs # Multi-drive watcher manager
│   ├── folder_compose.rs            # Folder cover composition (3-layer PNG)
│   ├── global_search.rs             # Named Pipe client for search IPC
│   ├── icon_disk_cache.rs           # Icon disk cache
│   ├── io_priority.rs               # I/O priority entry point
│   ├── io_priority/                 # I/O priority submodules
│   │   ├── detection.rs             # Storage type detection
│   │   ├── grouped_queue.rs         # Priority queue grouping
│   │   └── threading.rs             # Thread priority management
│   ├── ntfs_reader.rs               # Raw NTFS directory reading
│   ├── security.rs                  # Security validation entry point
│   ├── security/                    # Security submodules
│   │   ├── components.rs            # Security components
│   │   ├── drive.rs                 # Drive security
│   │   ├── shell_namespace.rs       # Shell namespace validation
│   │   ├── symlink.rs               # Symlink validation
│   │   └── unc.rs                   # UNC path validation
│   ├── shell_menu_worker.rs         # Shell context menu extraction
│   ├── user_session_search.rs       # User session search index
│   ├── virtual_drive_config.rs      # Virtual drive configuration
│   ├── windows_clipboard.rs         # Windows clipboard (CF_HDROP)
│   ├── onedrive/                    # OneDrive integration
│   │   ├── mod.rs
│   │   ├── attributes.rs            # OneDrive file attributes
│   │   ├── directory_enum.rs        # OneDrive directory enumeration
│   │   ├── path_detection.rs        # OneDrive path detection
│   │   ├── pin_state.rs             # OneDrive pin commands
│   │   └── timeout_ops.rs           # Timeout operations
│   ├── media/                       # Media infrastructure
│   │   ├── mod.rs
│   │   └── hardware_acceleration.rs # HW acceleration detection
│   └── windows/                     # Windows-specific APIs
│       ├── mod.rs
│       ├── bitmap_conversion.rs     # Bitmap conversion
│       ├── codec_registry.rs        # Codec name cache
│       ├── codec_registry/          # Codec registry submodules
│       ├── device_change.rs         # Device change monitoring
│       ├── drives.rs                # Drive enumeration
│       ├── file_flags.rs            # File attribute flags
│       ├── file_system.rs           # Filesystem operations
│       ├── file_type.rs             # File type detection
│       ├── folder_size.rs           # Folder size calculation
│       ├── formatting.rs            # Number/string formatting
│       ├── hdd_directory_reader.rs  # Optimized HDD directory reader
│       ├── icons.rs                 # Icon extraction
│       ├── icons/                   # Icon submodules
│       ├── iso_mount.rs             # ISO file mounting
│       ├── media_foundation.rs      # Media Foundation integration
│       ├── metadata/                # Metadata extraction submodules
│       ├── native_menu.rs           # Native Windows context menu
│       ├── recycle_bin.rs           # Recycle Bin operations
│       ├── recycle_bin/             # Recycle Bin submodules
│       ├── shell_folder.rs          # Shell special folders
│       ├── shell_operations.rs      # Shell file operations
│       ├── shell_operations/        # Shell operations submodules
│       ├── system_info.rs           # System information
│       ├── window_corners.rs        # Window corner styling
│       └── window_subclass.rs       # Window subclassing
│
├── ui/                              # User interface
│   ├── mod.rs
│   ├── app_impl.rs                  # eframe::App implementation
│   ├── cache.rs                     # Texture/icon cache manager
│   ├── theme.rs                     # UI theming
│   ├── widgets.rs                   # Custom egui widgets
│   ├── svg_icons.rs                 # SVG icon renderer
│   ├── toolbar.rs                   # Top toolbar
│   ├── sidebar.rs                   # Side panel
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
│   │   └── panels.rs                # Panel layout
│   ├── tab_bar/                     # Tab system
│   ├── preview_panel/               # Preview panel with video support
│   ├── icon_loader.rs               # Icon loading
│   ├── icon_loader/                 # Icon loader submodules
│   ├── global_search_overlay.rs     # Global search overlay
│   ├── global_search_overlay/       # Search overlay submodules
│   ├── components/                  # Reusable UI components
│   │   ├── mod.rs
│   │   ├── media_preview.rs         # Media preview component
│   │   ├── gif_manager.rs           # GIF playback manager
│   │   ├── video_controls_state.rs  # Video controls state
│   │   ├── language_settings.rs     # Language settings component
│   │   ├── virtual_drive_settings.rs # Virtual drive settings
│   │   ├── item_slot/               # Item slot rendering (drive/folder/file)
│   │   ├── mpv/                     # mpv integration
│   │   └── mpv_preview/             # mpv preview bridge
│   └── views/                       # File list views
│       ├── mod.rs
│       ├── common.rs                # Shared view utilities
│       ├── computer_view.rs         # "This PC" view
│       ├── grid_view/               # Grid view
│       └── list_view/               # List view
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
│           ├── stage1_image_crate.rs
│           ├── stage2_wic.rs
│           ├── stage3_shell_api.rs
│           ├── stage4_force_extract.rs
│           └── stage5_media_foundation.rs
│
├── image_viewer/                    # Dedicated image viewer (separate process)
│   ├── mod.rs                       # Process spawn & standalone runner
│   ├── app.rs                       # DedicatedImageViewerApp state
│   ├── cache.rs                     # WindowCache + PrefetchEngine
│   ├── indexer.rs                   # Image sequence builder
│   ├── ipc.rs                       # Inter-process communication
│   ├── loader.rs                    # Image decoding (mmap, EXIF, WIC)
│   └── metrics.rs                   # Performance metrics
│
├── video_player/                    # Standalone video player (separate process)
│   └── mod.rs                       # mpv-based player with D3D11 pipeline
│
└── pdf_viewer/                      # Native PDF viewer (separate process)
    ├── mod.rs                       # Process spawn, path validation
    ├── viewer_app.rs                # PDF viewer state & rendering
    ├── renderer.rs                  # Page rendering
    ├── render_worker.rs             # Async render worker
    └── toolbar.rs                   # PDF controls
```

## Workspace Crates

```
crates/
├── mtt-search-protocol/             # Shared IPC types
│   └── src/
│       └── lib.rs                    # SearchRequest, SearchResponse, bincode serialization
│
└── mtt-search-service/              # Windows Service for file indexing
    └── src/
        ├── main.rs                   # Entry point + SCM integration
        ├── usn_journal.rs            # Volume discovery + USN API (NTFS/ReFS)
        ├── fs_walker.rs              # Full-tree scanner for non-USN volumes
        ├── file_index.rs             # In-memory HashMap index
        ├── path_resolver.rs          # Path reconstruction via FRN chain
        ├── index_db.rs               # SQLite persistence
        ├── ipc_server.rs             # Named Pipe server
        ├── ipc_authorization.rs      # IPC authorization
        ├── security_policy.rs        # Security policy
        ├── service_control.rs        # Service install/uninstall
        ├── name_arena.rs             # String arena for name storage
        └── volume_indexers.rs        # Per-volume indexer management
```

## Configuration Files

```
Cargo.toml                           # Workspace root + mtt-file-manager dependencies
build.rs                             # Windows icon embedding (winresource)
virtual_drive_config.json            # Drive letter → storage type overrides
locales/
├── en.yml                           # English translations
└── pt-BR.yml                        # Brazilian Portuguese translations
assets/icons/gen/                    # Generated icon assets
mpv_ui/portable_config/              # mpv configuration
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
App State (src/app/state.rs) ←→ Application Services (src/application/)
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

