# Storage & Configuration — MTT File Manager

## Data Storage Locations

### Application Data (per-user)
```
%LOCALAPPDATA%\MTT-File-Manager\
├── thumbnails/
│   └── thumbnails.db          # Main SQLite database (all caches + preferences)
└── virtual_drive_config.json  # Virtual drive configuration (created on first app launch)
```

### Search Service Data (system-wide)
```
%PROGRAMDATA%\MTT-File-Manager\
└── search_index.db            # File index database (USN + full scan data)
```

## SQLite Schema: Application Database

Located at `%LOCALAPPDATA%\MTT-File-Manager\thumbnails\thumbnails.db`.

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

### Table: `user_preferences`
Stores user preferences as key-value pairs.

| Column | Type | Description |
|--------|------|-------------|
| `key` | TEXT (PK) | Preference key |
| `value` | TEXT | Preference value |

**Stored preference keys**: `sort_mode`, `sort_mode_computer`, `sort_mode_normal`, `sort_descending`, `folders_position`, `thumbnail_size`, `view_mode`, `show_preview_panel`, `upload_budget_ms`, `window_width`, `window_height`, `window_is_maximized`, `sidebar_left_width`, `sidebar_right_width`, `last_folder`, `media_volume`, `show_hidden_files`, `language`, `list_col_name_width`, `list_col_date_width`, `list_col_type_width`, `list_col_size_width`, `list_col_onedrive_name_width`, `list_col_onedrive_date_width`, `list_col_onedrive_type_width`, `list_col_onedrive_size_width`, `list_col_onedrive_status_width`, `list_col_computer_name_width`, `list_col_computer_total_width`, `list_col_computer_free_width`

### Table: `folder_covers`
Stores user-selected cover image for folders (which image to use as folder thumbnail).

| Column | Type | Description |
|--------|------|-------------|
| `folder_path` | TEXT (PK) | Folder path |
| `cover_path` | TEXT | Path to the image file used as cover |

### Table: `folder_previews`
Cached composed folder preview images (Shell sandwich icons).

| Column | Type | Description |
|--------|------|-------------|
| `folder_path` | TEXT (PK) | Folder path |
| `data` | BLOB | WebP-encoded preview image data |
| `width` | INTEGER | Image width |
| `height` | INTEGER | Image height |
| `created_at` | INTEGER | Cache timestamp |

### Table: `directory_index`
Cached directory metadata for fast folder size/count lookup.

| Column | Type | Description |
|--------|------|-------------|
| `dir_path` | TEXT (PK) | Directory path |
| `file_count` | INTEGER | Number of files |
| `total_size` | INTEGER | Total size of contents |
| `last_scan_time` | INTEGER | Timestamp of last scan |
| `scan_duration_ms` | INTEGER | Duration of the scan in milliseconds |

### Table: `file_index`
File-level index for cached directory contents.

| Column | Type | Description |
|--------|------|-------------|
| `id` | INTEGER (PK) | Auto-increment row ID |
| `dir_path` | TEXT | Parent directory path (indexed) |
| `file_name` | TEXT | File name |
| `file_size` | INTEGER | File size in bytes |
| `modified_time` | INTEGER | File modification timestamp |
| `is_dir` | INTEGER | Whether entry is a directory |

Unique constraint on `(dir_path, file_name)`.

### Table: `folder_locks`
Per-folder view preference overrides.

| Column | Type | Description |
|--------|------|-------------|
| `path` | TEXT (PK) | Folder path |
| `view_mode` | TEXT | Locked view mode (Grid/List) |
| `sort_mode` | TEXT | Locked sort mode (Name/Date/Size/Type) |
| `sort_descending` | TEXT | Locked sort direction ("true"/"false") |
| `folders_position` | TEXT | Locked folder position (First/Last/Mixed) |

### Table: `pinned_folders`
Quick Access pinned folder entries.

| Column | Type | Description |
|--------|------|-------------|
| `path` | TEXT (PK) | Folder path |
| `display_name` | TEXT | Display name in sidebar |
| `position` | INTEGER | Sort position for display order |

### Table: `shell_icons`
Cached Windows Shell icons (special folders, drives, "This PC", Recycle Bin). Stored as raw RGBA pixel data (not WebP).

| Column | Type | Description |
|--------|------|-------------|
| `key` | TEXT (PK) | Icon identifier (e.g. drive letter, CLSID) |
| `data` | BLOB | Raw RGBA pixel data |
| `width` | INTEGER | Image width |
| `height` | INTEGER | Image height |
| `created_at` | INTEGER | Cache timestamp |

## In-Memory Caches (not persisted)

### Directory Cache
`infrastructure/directory_cache.rs` provides an in-memory LRU cache (`lru::LruCache`) of directory contents (200 entries max). Not stored in SQLite — entries are populated on navigation and invalidated by the DriveWatcher on filesystem changes.

## SQLite Schema: Search Service Database

Located at `%PROGRAMDATA%\MTT-File-Manager\search_index.db`.

### Table: `volume_state`
Tracks indexing state per volume.

| Column | Type | Description |
|--------|------|-------------|
| `drive_letter` | TEXT (PK) | Drive letter (e.g., "C") |
| `journal_id` | INTEGER | USN Journal ID (for validation) |
| `last_usn` | INTEGER | Last processed USN |
| `files_indexed` | INTEGER | Number of indexed files |
| `last_full_scan_epoch` | INTEGER | Timestamp of last full scan |

### Table: `file_records`
File index entries for search.

| Column | Type | Description |
|--------|------|-------------|
| `frn` | INTEGER | File Reference Number |
| `drive_letter` | TEXT | Drive letter |
| `name` | TEXT | File/folder name |
| `parent_frn` | INTEGER | Parent directory FRN |
| `is_dir` | INTEGER | Whether entry is a directory |

Primary key: `(drive_letter, frn)`.

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
- Thumbnails are encoded as WebP (lossy, quality 85) and stored as BLOBs in the SQLite database (`thumbnails.db`)
- No individual image files are written to disk — everything lives inside SQLite
- Primary key is a BLAKE3 hash (128-bit / 32 hex chars) of the original file path
- Images larger than 1024x1024 are resized down before storage; alpha channel is preserved when present

### Invalidation
- File modification time change → re-generate
- Folder cover: staleness detection via content mtime comparison

### Cache Cleaning
```powershell
# Remove entire cache (thumbnails, preferences, everything)
Remove-Item "$env:LOCALAPPDATA\MTT-File-Manager" -Recurse -Force

# Remove only the SQLite database (clears thumbnails + folder previews)
Remove-Item "$env:LOCALAPPDATA\MTT-File-Manager\thumbnails\thumbnails.db"
```

## Icon Caching

Shell icons (special folders, drives, "This PC", Recycle Bin) are cached as raw RGBA pixel data in the `shell_icons` table via `infrastructure/disk_cache/shell_icons.rs`. Only successful extractions are stored. Icons are typically ~256x256 RGBA (~256 KB each), so no compression is applied.

## Preferences Persistence

User preferences are loaded during app initialization (`app/init_preferences.rs`) and saved to the SQLite `user_preferences` table on changes. Writes are debounced (dirty flag + 1-second flush interval) to avoid blocking the UI thread. On exit, a blocking flush ensures all pending changes are persisted.

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
- List view column widths (regular, OneDrive, computer views)
- Upload budget (GPU texture upload time per frame)

## Internationalization

Locale files are stored in the `locales/` directory:
- `locales/en.yml` — English
- `locales/pt-BR.yml` — Brazilian Portuguese (fallback and default)

The `rust-i18n` crate loads translations at compile time with `fallback = "pt-BR"`. Language preference is persisted in the SQLite `user_preferences` table and applied on startup. Default language when no preference is saved: `pt-BR`.

