# Storage & Configuration — MTT File Manager

## Data Storage Locations

### Application Data (per-user)
```
%LOCALAPPDATA%\MTT-File-Manager\
├── thumbnails/
│   └── thumbnails.db          # Thumbnail / folder preview / shell icon cache
├── state/
│   └── app_state.db           # Preferences, folder locks, pinned folders, folder covers
├── cache/
│   └── directory_cache.db     # Persisted directory metadata cache
└── virtual_drive_config.json  # Virtual drive configuration (created on first app launch)
```

### Search Service Data (system-wide)
```
%PROGRAMDATA%\MTT-File-Manager\
└── search_index.db            # File index database (USN + full scan data)
```

**Security note**: The search service hardcodes `C:\ProgramData` instead of reading the `%PROGRAMDATA%` environment variable to prevent env-var redirection attacks. The directory ACL is hardened via `SetSecurityInfo` on the kernel handle (not by path), and the directory is validated to not be a reparse point before ACL application to prevent junction-planting attacks.

## SQLite Schema: Thumbnail Cache Database

Located at `%LOCALAPPDATA%\MTT-File-Manager\thumbnails\thumbnails.db`.

Connection management uses a dual writer/reader pattern with WAL mode. If the primary path fails (e.g., ACL hardening failure), the system falls back to a temporary database in `%TEMP%\MTT-File-Manager\thumbnails_fallback.db`.

### Table: `thumbnails`
Stores cached thumbnails as WebP-encoded BLOBs.

| Column | Type | Description |
|--------|------|-------------|
| `id` | TEXT (PK) | BLAKE3 hash of the file path (first 128 bits, 32 hex chars) |
| `path` | TEXT | Original file path (indexed for directory-level clearing) |
| `data` | BLOB | WebP-encoded thumbnail image data |
| `modified_at` | INTEGER | File modification time (for invalidation) |
| `created_at` | INTEGER | Cache entry creation timestamp |
| `width` | INTEGER | Thumbnail width |
| `height` | INTEGER | Thumbnail height |
| `requested_size` | INTEGER | Requested thumbnail size at generation time |

**Migrations**: The schema auto-migrates to add `path`, `width`, `height`, and `requested_size` columns if they are missing from older databases.

### Table: `folder_previews`
Cached composed folder preview images (Shell sandwich icons).

| Column | Type | Description |
|--------|------|-------------|
| `folder_path` | TEXT (PK) | Folder path |
| `data` | BLOB NOT NULL | WebP-encoded preview image data |
| `width` | INTEGER NOT NULL | Image width |
| `height` | INTEGER NOT NULL | Image height |
| `created_at` | INTEGER NOT NULL | Cache timestamp |

### Table: `shell_icons`
Cached Windows Shell icons (special folders, drives, "This PC", Recycle Bin). Stored as raw RGBA pixel data (not WebP).

| Column | Type | Description |
|--------|------|-------------|
| `key` | TEXT (PK) | Icon identifier (e.g. drive letter, CLSID) |
| `data` | BLOB NOT NULL | Raw RGBA pixel data |
| `width` | INTEGER NOT NULL | Image width |
| `height` | INTEGER NOT NULL | Image height |
| `created_at` | INTEGER NOT NULL | Cache timestamp |

## SQLite Schema: App State Database

Located at `%LOCALAPPDATA%\MTT-File-Manager\state\app_state.db`.

On upgrade, `app/init_bootstrap.rs` runs a one-time migration that copies the legacy `user_preferences`, `folder_covers`, `folder_locks`, and `pinned_folders` tables out of the old monolithic `thumbnails.db` into `app_state.db`, then drops the legacy copies from `thumbnails.db`.

### Table: `user_preferences`
Stores user preferences as key-value pairs.

| Column | Type | Description |
|--------|------|-------------|
| `key` | TEXT (PK) | Preference key |
| `value` | TEXT | Preference value |

**Stored preference keys**: `sort_mode`, `sort_mode_computer`, `sort_mode_normal`, `sort_descending`, `folders_position`, `thumbnail_size`, `view_mode`, `show_preview_panel`, `upload_budget_ms`, `window_width`, `window_height`, `window_is_maximized`, `sidebar_left_width`, `sidebar_right_width`, `last_folder`, `media_volume`, `show_hidden_files`, `language`, `theme_mode`, `gpu_backend`, `list_col_name_width`, `list_col_date_width`, `list_col_type_width`, `list_col_size_width`, `list_col_onedrive_name_width`, `list_col_onedrive_date_width`, `list_col_onedrive_type_width`, `list_col_onedrive_size_width`, `list_col_onedrive_status_width`, `list_col_computer_name_width`, `list_col_computer_total_width`, `list_col_computer_free_width`

### Table: `folder_covers`
Stores user-selected cover image for folders (which image to use as folder thumbnail).

| Column | Type | Description |
|--------|------|-------------|
| `folder_path` | TEXT (PK) | Folder path |
| `cover_path` | TEXT | Path to the image file used as cover |

### Table: `folder_locks`
Per-folder view preference overrides.

| Column | Type | Description |
|--------|------|-------------|
| `path` | TEXT (PK) | Folder path |
| `view_mode` | TEXT NOT NULL | Locked view mode (Grid/List) |
| `sort_mode` | TEXT NOT NULL | Locked sort mode (Name/Date/Size/Type) |
| `sort_descending` | TEXT NOT NULL | Locked sort direction ("true"/"false") |
| `folders_position` | TEXT NOT NULL | Locked folder position (First/Last/Mixed) |

**Migration note**: The legacy `folder_locks` table had a `search_query NOT NULL` column that caused INSERT failures. The migration drops and recreates the table without that column.

### Table: `pinned_folders`
Quick Access pinned folder entries.

| Column | Type | Description |
|--------|------|-------------|
| `path` | TEXT (PK) | Folder path |
| `display_name` | TEXT NOT NULL | Display name in sidebar |
| `position` | INTEGER NOT NULL DEFAULT 0 | Sort position for display order |

## SQLite Schema: Directory Cache Database

Located at `%LOCALAPPDATA%\MTT-File-Manager\cache\directory_cache.db`.

### Table: `directory_index`
Cached directory metadata for fast folder size/count lookup.

| Column | Type | Description |
|--------|------|-------------|
| `dir_path` | TEXT (PK) | Directory path |
| `file_count` | INTEGER NOT NULL | Number of files |
| `total_size` | INTEGER NOT NULL | Total size of contents |
| `last_scan_time` | INTEGER NOT NULL | Timestamp of last scan |
| `scan_duration_ms` | INTEGER NOT NULL | Duration of the scan in milliseconds |

### Table: `file_index`
File-level index for cached directory contents.

| Column | Type | Description |
|--------|------|-------------|
| `id` | INTEGER (PK) | Auto-increment row ID |
| `dir_path` | TEXT NOT NULL | Parent directory path (indexed) |
| `file_name` | TEXT NOT NULL | File name |
| `file_size` | INTEGER NOT NULL | File size in bytes |
| `modified_time` | INTEGER NOT NULL | File modification timestamp |
| `is_dir` | INTEGER NOT NULL | Whether entry is a directory |

Unique constraint on `(dir_path, file_name)`.

## In-Memory Caches (not persisted)

### Directory Cache
`infrastructure/directory_cache.rs` provides an in-memory LRU cache (`lru::LruCache`) of directory contents (200 entries max). Not stored in SQLite — entries are populated on navigation and invalidated by filesystem watcher events on changes.

## SQLite Schema: Session Search Database

Located at `%LOCALAPPDATA%\MTT-File-Manager\thumbnails\session_search.db`.

Used by the `user_session_search/` module for the in-app session search index.

### Table: `session_items`

| Column | Type | Description |
|--------|------|-------------|
| `drive_letter` | TEXT NOT NULL | Drive letter |
| `name` | TEXT NOT NULL | File/folder name |
| `full_path` | TEXT NOT NULL | Full path |
| `is_dir` | INTEGER NOT NULL | Whether entry is a directory |

Indexed on `drive_letter`.

### Table: `session_volumes`

| Column | Type | Description |
|--------|------|-------------|
| `drive_letter` | TEXT (PK) | Drive letter |
| `label` | TEXT NOT NULL | Volume label |
| `file_system` | TEXT NOT NULL | File system type |

## SQLite Schema: Search Service Database

Located at `%PROGRAMDATA%\MTT-File-Manager\search_index.db`.

### Table: `volume_state`
Tracks indexing state per volume.

| Column | Type | Description |
|--------|------|-------------|
| `drive_letter` | TEXT (PK) | Drive letter (e.g., "C") |
| `journal_id` | INTEGER NOT NULL | USN Journal ID (for validation) |
| `last_usn` | INTEGER NOT NULL | Last processed USN |
| `files_indexed` | INTEGER NOT NULL | Number of indexed files |
| `last_full_scan_epoch` | INTEGER NOT NULL | Timestamp of last full scan |
| `has_hardlink_parent_data` | INTEGER NOT NULL DEFAULT 0 | Whether `hardlink_parents` is populated for this volume |
| `has_reparse_point_data` | INTEGER NOT NULL DEFAULT 0 | Whether reparse point metadata was captured |

### Table: `file_records`
File index entries for search.

| Column | Type | Description |
|--------|------|-------------|
| `frn` | INTEGER NOT NULL | File Reference Number |
| `drive_letter` | TEXT NOT NULL | Drive letter |
| `name` | TEXT NOT NULL | File/folder name |
| `parent_frn` | INTEGER NOT NULL | Parent directory FRN |
| `is_dir` | INTEGER NOT NULL | Whether entry is a directory |
| `is_reparse` | INTEGER NOT NULL DEFAULT 0 | Whether the entry is a reparse point |

Primary key: `(drive_letter, frn)`.

### Table: `hardlink_parents`
Stores additional parent directories for hardlinked files.

| Column | Type | Description |
|--------|------|-------------|
| `drive_letter` | TEXT NOT NULL | Drive letter |
| `frn` | INTEGER NOT NULL | File Reference Number |
| `parent_frn` | INTEGER NOT NULL | Additional parent directory FRN |

Primary key: `(drive_letter, frn, parent_frn)`.

### Binary snapshot: `index_<drive>.bin`
Per-volume binary cache stored alongside `search_index.db` under `C:\ProgramData\MTT-File-Manager`.

Used as the primary fast-start cache for USN volumes. The service loads this file before falling back to SQLite rows.

Layout:
- Header (72 bytes): magic, version, drive letter, journal metadata, entry counts, flags
- NameArena bytes
- Packed records (`FRN + FileRecord`)
- Hardlink parent pairs
- Reparse-point FRNs
- CRC32 trailer

### Virtual table: `search_fts` (legacy schema)
Legacy FTS5 trigram index over `file_records.name`, still present in the SQLite schema for compatibility with older persistence/rebuild code paths.

Current live search queries no longer depend on this table; they run against the in-memory lowered `NameArena` instead.

### Table: `service_meta`
Stores service-wide metadata.

| Column | Type | Description |
|--------|------|-------------|
| `key` | TEXT (PK) | Metadata key |
| `value` | INTEGER NOT NULL | Metadata value |

Currently used key:
- `dirty` — set to `1` on startup and cleared to `0` on clean shutdown; if startup sees `dirty=1`, the service rebuilds the legacy `search_fts` table before continuing SQLite maintenance.

### Schema migrations in search service
The service automatically migrates the schema on startup:
- **7-column → 5-column**: Old `file_records` schema with 7 columns is replaced by the compact 5-column schema (dropping `size` and `mtime` columns)
- **`has_hardlink_parent_data`**: Added to `volume_state` if missing
- **`has_reparse_point_data`**: Added to `volume_state` if missing
- **`is_reparse`**: Added to `file_records` if missing

### Runtime behavior
- USN volumes prefer `index_<drive>.bin` for startup and fall back to SQLite only when the binary cache is missing, stale, or invalid.
- Full USN scans write a new binary snapshot after `read_mft_bulk()` completes; periodic USN catch-up persists `volume_state` in SQLite while keeping the in-memory index authoritative.
- Non-USN volumes persist `file_records` and `hardlink_parents` to SQLite after full-tree scans; those rows also serve as their fast-start cache.
- Search IPC requests are served from the in-memory index using the lowered `NameArena`; SQLite is not on the hot query path.
- The SQLite `search_fts` table remains a legacy persisted artifact. Some persistence paths still rebuild or update it, but live query correctness and performance no longer depend on it.

## Virtual Drive Configuration

File: `%LOCALAPPDATA%\MTT-File-Manager\virtual_drive_config.json`

```json
{
  "overrides": {
    "X": "HDD",
    "Y": "HDD",
    "Z": "HDD"
  }
}
```

Maps drive letters to storage type hints. Affects which directory reading strategy is used:
- **SSD**: Standard directory enumeration
- **HDD**: Optimized `NtQueryDirectoryFile` bulk reading

Behavior:
- The app creates this file automatically on first launch if it does not exist.
- During creation, the app scans currently available virtual drives and writes them into the file.
- Newly detected virtual drives default to **SSD** until the user changes them in the status bar optimization settings.
- The settings window reads and writes this same file in `%LOCALAPPDATA%`.

## Thumbnail Disk Cache

### Cache Structure
- `thumbnails.db` stores only `thumbnails`, `folder_previews`, and `shell_icons`
- App preferences and per-folder UI state live in `state/app_state.db`
- Persisted directory metadata lives in `cache/directory_cache.db`
- Thumbnails are encoded as WebP (lossy, quality 85) and stored as BLOBs in `thumbnails.db`
- No individual thumbnail or folder-preview image files are written to disk — everything lives inside SQLite
- Primary key is a BLAKE3 hash (128-bit / 32 hex chars) of the original file path
- Images larger than 1024x1024 are resized down before storage; alpha channel is preserved when present

### Invalidation
- File modification time change → re-generate
- Folder cover: staleness detection via content mtime comparison

### Cache Cleaning
```powershell
# Remove all per-user data (thumbnail cache + app state + directory cache + config)
Remove-Item "$env:LOCALAPPDATA\MTT-File-Manager" -Recurse -Force

# Remove only the thumbnail / folder preview SQLite cache
Remove-Item "$env:LOCALAPPDATA\MTT-File-Manager\thumbnails\thumbnails.db"

# Remove only app preferences and per-folder state
Remove-Item "$env:LOCALAPPDATA\MTT-File-Manager\state\app_state.db"

# Remove only the persisted directory metadata cache
Remove-Item "$env:LOCALAPPDATA\MTT-File-Manager\cache\directory_cache.db"
```

## Icon Caching

Shell icons (special folders, drives, "This PC", Recycle Bin) are cached as raw RGBA pixel data in the `shell_icons` table via `infrastructure/disk_cache/shell_icons.rs`. Only successful extractions are stored. Icons are typically ~256x256 RGBA (~256 KB each), so no compression is applied.

## Preferences Persistence

User preferences are loaded during app initialization (`app/init_preferences.rs`) and saved to the `user_preferences` table in `%LOCALAPPDATA%\MTT-File-Manager\state\app_state.db`. Writes are debounced (dirty flag + 1-second flush interval) to avoid blocking the UI thread. On exit, a blocking flush ensures all pending changes are persisted.

**Saved preferences include**:
- Sort mode (global, computer view, normal view) and direction
- View mode (Grid/List)
- Thumbnail size
- Window dimensions, position, and maximized state
- Left and right sidebar widths
- Preview panel visibility
- Folders position (First/Last/Mixed)
- Last active folder
- Media volume
- Show hidden files toggle
- Language selection (en, pt-BR)
- Theme mode (Light/Dark)
- GPU backend preference (dx12, vulkan, gl, auto)
- List view column widths (regular, OneDrive, computer views)
- Upload budget (GPU texture upload time per frame)

## Internationalization

Locale files are stored in the `locales/` directory:
- `locales/en.yml` — English
- `locales/pt-BR.yml` — Brazilian Portuguese (fallback and default)

The `rust-i18n` crate loads translations at compile time with `fallback = "pt-BR"`. Language preference is persisted in the `user_preferences` table inside `%LOCALAPPDATA%\MTT-File-Manager\state\app_state.db` and applied on startup. Default language when no preference is saved: `pt-BR`.
