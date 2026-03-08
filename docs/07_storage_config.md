# Storage & Configuration — MTT File Manager

## Data Storage Locations

### Application Data (per-user)
```
%LOCALAPPDATA%\MTT-File-Manager\
├── thumbnails/
│   ├── thumbnails.db          # Main SQLite database
│   └── *.webp                 # Thumbnail cache files (WebP format)
└── virtual_drive_config.json  # Virtual drive configuration (copied from project)
```

### Search Service Data (system-wide)
```
%PROGRAMDATA%\MTT-File-Manager\
└── search_index.db            # File index database (USN + full scan data)
```

## SQLite Schema: Application Database

Located at `%LOCALAPPDATA%\MTT-File-Manager\thumbnails\thumbnails.db`.

### Table: `thumbnails`
Stores cached thumbnail metadata and file references.

| Column | Type | Description |
|--------|------|-------------|
| `path_hash` | TEXT (PK) | BLAKE3 hash of the file path |
| `file_path` | TEXT | Original file path |
| `thumbnail_path` | TEXT | Path to the cached WebP file |
| `file_size` | INTEGER | Original file size (for invalidation) |
| `modified_time` | INTEGER | File modification time (for invalidation) |
| `width` | INTEGER | Thumbnail width |
| `height` | INTEGER | Thumbnail height |
| `created_at` | INTEGER | Cache entry creation timestamp |

### Table: `preferences`
Stores user preferences as key-value pairs.

| Column | Type | Description |
|--------|------|-------------|
| `key` | TEXT (PK) | Preference key |
| `value` | TEXT | Preference value |

**Stored preferences include**: sort_mode, thumbnail_size, view_mode, window position/size, sidebar widths, preview panel state, language, folders_position

### Table: `directory_cache`
Caches directory metadata for fast access.

| Column | Type | Description |
|--------|------|-------------|
| `path` | TEXT (PK) | Directory path |
| `file_count` | INTEGER | Number of files |
| `total_size` | INTEGER | Total size of contents |
| `cached_at` | INTEGER | Cache timestamp |

### Table: `folder_locks`
Per-folder view preference overrides.

| Column | Type | Description |
|--------|------|-------------|
| `path` | TEXT (PK) | Folder path |
| `view_mode` | TEXT | Locked view mode (Grid/List) |
| `sort_mode` | TEXT | Locked sort mode (Name/Date/Size/Type) |
| `folders_position` | TEXT | Locked folder position (First/Last/Mixed) |

### Table: `pinned_folders`
Quick Access pinned folder entries.

| Column | Type | Description |
|--------|------|-------------|
| `path` | TEXT (PK) | Folder path |
| `display_name` | TEXT | Display name in sidebar |
| `position` | INTEGER | Sort position for display order |

### Table: `folder_previews`
Cached composed folder cover images.

| Column | Type | Description |
|--------|------|-------------|
| `path_hash` | TEXT (PK) | Hash of the folder path |
| `image_data` | BLOB | Composed cover image data |
| `created_at` | INTEGER | Cache timestamp |

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

## Virtual Drive Configuration

File: `virtual_drive_config.json` (project root, copied to `%LOCALAPPDATA%` at runtime)

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

## Thumbnail Disk Cache

### Cache Structure
- Thumbnails are stored as WebP files in `%LOCALAPPDATA%\MTT-File-Manager\thumbnails\`
- File names are BLAKE3 hashes of the original file path
- Metadata (path, size, timestamps) in SQLite for invalidation checks

### Invalidation
- File size or modification time change → re-generate
- Folder cover: staleness detection via content mtime comparison

### Cache Cleaning
```powershell
# Remove entire cache (thumbnails, preferences, everything)
Remove-Item "$env:LOCALAPPDATA\MTT-File-Manager" -Recurse -Force

# Remove only thumbnail images (keeps preferences/locks)
Remove-Item "$env:LOCALAPPDATA\MTT-File-Manager\thumbnails\*.webp"
```

## Icon Caching

Icons extracted from the Windows Shell are cached in the SQLite database via `infrastructure/disk_cache/shell_icons.rs`. The icon cache tracks both successful extractions and failed attempts to avoid repeatedly trying to extract icons from files that don't provide them.

## Preferences Persistence

User preferences are loaded during app initialization (`app/init_preferences.rs`) and saved to the SQLite `preferences` table on changes. Includes:

- Sort mode and direction
- View mode (Grid/List)
- Thumbnail size
- Window position and dimensions
- Sidebar width
- Preview panel visibility and width
- Folders position (First/Last/Mixed)
- Language selection (en, pt-BR)

## Internationalization

Locale files are stored in the `locales/` directory:
- `locales/en.yml` — English
- `locales/pt-BR.yml` — Brazilian Portuguese (fallback)

The `rust-i18n` crate loads translations at compile time. Language preference is persisted in the SQLite `preferences` table and applied on startup.

