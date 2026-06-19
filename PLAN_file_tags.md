# File Tags / Color Labels - Revised Implementation Plan

## Goal

Add path-based file tags to MTT File Manager. Users can assign one or more colored tags to real files and folders, see tag indicators in grid/list views, filter the current panel by tag, and manage tag definitions through the app UI.

## Scope

### In Scope

- Tag definitions persisted in the existing `app_state.db`.
- Many-to-many assignments from filesystem paths to tags.
- Grid/list visual indicators for tagged items.
- Tag submenu in the existing context menu for selected real files/folders.
- Sidebar Tags section with counts and click-to-filter behavior.
- Basic tag manager modal: create, rename, recolor, delete.
- Per-tab and per-dual-panel active tag filter.
- Cleanup of assignments for deleted paths and best-effort preservation for app-initiated rename/move.

### Out Of Scope For V1

- Global search indexing by tag.
- NTFS Alternate Data Streams or shell property integration.
- Tags that follow files moved outside this app.
- Tagging virtual locations such as This PC, Recycle Bin items, shell namespace paths, archive-internal virtual entries, and drive roots.
- Persisting full multi-tab sessions. Only the active tab/filter preference can be restored through existing preferences.

## Codebase Findings That Affect The Plan

- `AppStateDb` already owns `app_state.db` and migrations in `src/infrastructure/app_state_db/mod.rs`; existing table helpers live beside it, for example `pinned_folders.rs`, `folder_locks.rs`, and `folder_covers.rs`.
- `db_utils::apply_default_pragmas()` currently sets WAL and synchronous mode only. SQLite foreign-key cascades are not active unless `PRAGMA foreign_keys = ON` is added per connection or deletes are done explicitly.
- Startup preferences are loaded in one batch by `StartupPreferences::load()` in `src/app/init_preferences.rs`, then saved by `collect_preferences()` in `src/app/operations/preferences.rs`.
- Grid/list rendering is bridged through `src/app/operations/ui_rendering/grid_bridge.rs` and `src/app/operations/ui_rendering/list_bridge.rs`, not directly from `ImageViewerApp` into leaf widgets only.
- `ContextMenuItem` supports submenus and icons, but does not currently support checkmarks or color-dot metadata.
- Filtering is used in both UI-thread paths (`folder_loading/view_updates.rs`) and background rebuild paths (`message_handler/thumbnail_rebuild.rs`), so tag filtering must be wired into both.
- Sidebar rendering is split between `src/ui/sidebar.rs` and `src/ui/app/panels/mod.rs`; actions are handled in `handle_sidebar_action()`.
- Tag manager modal should be rendered from `src/ui/app_impl.rs`, similar to settings and batch rename modals, not from `src/ui/app/layers.rs`.

## Data Model

### Domain Types

Create `src/domain/file_tag.rs`:

```rust
pub enum TagColor {
    Red,
    Orange,
    Yellow,
    Green,
    Blue,
    Purple,
    Gray,
}

pub struct FileTag {
    pub id: i64,
    pub name: String,
    pub color: TagColor,
    pub position: i64,
}
```

Required helpers:

- `TagColor::as_db_str() -> &'static str`
- `TagColor::from_db_str(&str) -> Option<TagColor>`
- `TagColor::to_color32() -> egui::Color32`
- `TagColor::default_palette() -> [TagColor; 7]`

Use fixed colors:

| Color | RGB |
|---|---|
| Red | `255, 59, 48` |
| Orange | `255, 149, 0` |
| Yellow | `255, 204, 0` |
| Green | `52, 199, 89` |
| Blue | `0, 122, 255` |
| Purple | `175, 82, 222` |
| Gray | `142, 142, 147` |

Register the module in `src/domain/mod.rs`.

### SQLite Schema

Add tables in `AppStateDb::run_migrations()`:

```sql
CREATE TABLE IF NOT EXISTS file_tags (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL COLLATE NOCASE UNIQUE,
    color TEXT NOT NULL,
    position INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS file_tag_assignments (
    file_path TEXT NOT NULL,
    tag_id INTEGER NOT NULL,
    PRIMARY KEY (file_path, tag_id),
    FOREIGN KEY (tag_id) REFERENCES file_tags(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_file_tag_assignments_tag
    ON file_tag_assignments(tag_id);
```

Implementation notes:

- Do not depend only on `ON DELETE CASCADE` unless `PRAGMA foreign_keys = ON` is added to both `writer_conn` and `reader_conn` after opening them.
- Even with the FK, `delete_tag()` should explicitly delete assignments in the same writer transaction before deleting the tag. This matches the current defensive DB style and avoids surprises if a connection opens without foreign keys enabled.
- Seed the seven default tags only when `file_tags` is empty. Use stable default names (`Red`, `Orange`, `Yellow`, `Green`, `Blue`, `Purple`, `Gray`) because migrations run before locale is applied. Users can rename them.
- Do not call `canonicalize()` or blocking filesystem metadata APIs while assigning/rendering tags. Store the path string already present in `FileEntry.path`.

## Runtime State

Add to `ImageViewerApp` in `src/app/state/mod.rs`:

```rust
pub tag_definitions: rustc_hash::FxHashMap<i64, FileTag>,
pub tag_assignments: std::sync::Arc<rustc_hash::FxHashMap<PathBuf, Vec<i64>>>,
pub tag_counts: rustc_hash::FxHashMap<i64, usize>,
pub active_tag_filter: Option<i64>,
pub collapse_tags: bool,
pub show_tag_manager: bool,
```

Reasoning:

- `tag_assignments` should be an `Arc<FxHashMap<...>>` so background rebuild jobs can capture a cheap snapshot.
- Mutations can use `Arc::make_mut(&mut self.tag_assignments)` in tag operations.
- `tag_counts` avoids sidebar COUNT queries during rendering and can be updated incrementally on assignment changes.
- `active_tag_filter` is per active tab/panel state, not global rendering state.

## Persistence API

Create `src/infrastructure/app_state_db/file_tags.rs` and register it in `src/infrastructure/app_state_db/mod.rs`.

Add `impl AppStateDb` methods:

| Method | Connection | Purpose |
|---|---|---|
| `get_all_tags() -> Vec<FileTag>` | reader | Load definitions ordered by `position, name` |
| `ensure_default_tags()` | writer | Insert default tags if table is empty |
| `create_tag(name: &str, color: TagColor) -> Option<i64>` | writer | Insert a custom tag |
| `rename_tag(id: i64, name: &str) -> bool` | writer | Rename, respecting `COLLATE NOCASE UNIQUE` |
| `update_tag_color(id: i64, color: TagColor) -> bool` | writer | Recolor a tag |
| `delete_tag(id: i64) -> bool` | writer | Delete assignments, then definition |
| `get_all_tag_assignments() -> FxHashMap<PathBuf, Vec<i64>>` | reader | Startup load |
| `assign_tag(path: &Path, tag_id: i64) -> bool` | writer | `INSERT OR IGNORE` |
| `unassign_tag(path: &Path, tag_id: i64) -> bool` | writer | Delete one assignment |
| `clear_tag_assignments_for_paths(paths: &[PathBuf]) -> usize` | writer | Delete cleanup |
| `move_tag_assignments(old_path: &Path, new_path: &Path) -> bool` | writer | Preserve tags on app rename/move |
| `get_tag_counts() -> FxHashMap<i64, usize>` | reader | Initial count load or diagnostics |

Keep DB methods non-panicking and follow existing logging style in `pinned_folders.rs` and `folder_locks.rs`.

## Implementation Phases

### Phase 1 - Domain And DB Foundation

Files:

- `src/domain/file_tag.rs`
- `src/domain/mod.rs`
- `src/infrastructure/app_state_db/mod.rs`
- `src/infrastructure/app_state_db/file_tags.rs`

Tasks:

1. Add `TagColor` and `FileTag`.
2. Add SQLite tables and indexes.
3. Add default tag seeding.
4. Add DB CRUD and assignment helpers.
5. Decide explicitly whether to enable `PRAGMA foreign_keys = ON`; do not silently rely on cascades.

Verification:

- Add focused unit tests where practical for color parsing and DB CRUD.
- Run `cargo fmt` and `cargo check`.

### Phase 2 - App State, Startup, Preferences, Tabs

Files:

- `src/app/state/mod.rs`
- `src/app/init.rs`
- `src/app/init_preferences.rs`
- `src/app/operations/preferences.rs`
- `src/tabs/mod.rs`
- `src/app/operations/tabs.rs`
- `src/app/dual_panel.rs`

Tasks:

1. Add tag state fields to `ImageViewerApp` and initialize them in `ImageViewerApp::new()`.
2. After `rust_i18n::set_locale(&language)`, load tag definitions, assignments, and counts from `app_state_db`.
3. Add `active_tag_filter` to `StartupPreferences` only if restoring the last active filter is desired.
4. Persist `active_tag_filter` through `collect_preferences()` and validate the restored ID still exists.
5. Add `active_tag_filter` and `collapse_tags` to `TabState`, constructors, duplicate/reopen paths, `sync_to_tab()`, and `sync_from_tab()`.
6. Add `active_tag_filter` to `PanelSnapshot::from_app()`, `apply_to()`, and `swap_with_app()` so dual panels do not leak filters into each other.

Important detail:

- `tag_definitions`, `tag_assignments`, and `tag_counts` are app-wide metadata and should not be duplicated into every tab. Only UI filter/collapse state belongs in tabs/panels.

### Phase 3 - Tag Operations

Files:

- `src/app/operations/tag_ops.rs`
- `src/app/operations/mod.rs`
- `src/app/operations/message_handler/file_op_events.rs`

Tasks:

1. Register `pub mod tag_ops;`.
2. Add `ImageViewerApp` methods:

```rust
assign_tag_to_paths(&mut self, paths: &[PathBuf], tag_id: i64)
unassign_tag_from_paths(&mut self, paths: &[PathBuf], tag_id: i64)
toggle_tag_on_paths(&mut self, paths: &[PathBuf], tag_id: i64)
create_new_tag(&mut self, name: &str, color: TagColor) -> Option<i64>
rename_tag_definition(&mut self, tag_id: i64, name: &str) -> bool
update_tag_definition_color(&mut self, tag_id: i64, color: TagColor) -> bool
set_tag_filter(&mut self, tag_id: Option<i64>)
paths_have_tag(&self, paths: &[PathBuf], tag_id: i64) -> bool
paths_tag_ids(&self, paths: &[PathBuf]) -> Vec<i64>
```

3. Update in-memory assignments and counts before/after DB writes consistently. If a DB write fails, either roll back the in-memory change or reload tags from DB.
4. Call `filter_items()` and request repaint after changing `active_tag_filter` or assignments that affect visible items.
5. On `RenameCompleted`, move assignments from the old path to the new path.
6. On `MoveCompleted` and `MoveBatchCompleted`, preserve tags best-effort by mapping `source_folder.join(file_name)` to `dest_folder.join(file_name)` when the destination path is known.
7. On `DeleteCompleted`, clear assignments for deleted paths from DB, memory, and counts.

Behavior decision:

- V1 should preserve tags on app-initiated rename/move.
- V1 should not automatically copy tags on copy operations unless explicitly requested later.

### Phase 4 - Filtering Pipeline

Files:

- `src/application/sorting.rs`
- `src/application/sorting/filtering.rs`
- `src/app/operations/folder_loading/view_updates.rs`
- `src/app/operations/message_handler/thumbnail_rebuild.rs`

Tasks:

1. Add a filtering API that accepts `active_tag_filter` and `tag_assignments`.
2. Apply name filtering and tag filtering in one pass when possible.
3. Keep the existing `filter_items_opt()` for callers that do not need tags, or replace all call sites deliberately.
4. Update background rebuild jobs to capture `active_tag_filter` and `Arc` clone of `tag_assignments`.
5. Sort after filtering exactly as today.

Suggested API:

```rust
pub fn filter_items_opt_with_tags(
    items: &[FileEntry],
    query: &str,
    active_tag_filter: Option<i64>,
    tag_assignments: &FxHashMap<PathBuf, Vec<i64>>,
) -> Option<Vec<FileEntry>>
```

Expected behavior:

- If no query and no tag filter, return `None` to preserve the current no-clone fast path.
- If only a tag filter is active, return tagged items only.
- If both are active, require both conditions.

### Phase 5 - Grid And List Indicators

Files:

- `src/ui/components/item_slot/mod.rs`
- `src/ui/components/item_slot/badges.rs`
- `src/ui/components/item_slot/file_slot.rs`
- `src/ui/components/item_slot/folder_slot.rs`
- `src/ui/views/grid_view/mod.rs`
- `src/ui/views/grid_view/item_renderer.rs`
- `src/app/operations/ui_rendering/grid_bridge.rs`
- `src/app/operations/ui_rendering/item_slot_bridge.rs`
- `src/ui/views/list_view/mod.rs`
- `src/ui/views/list_view/item_renderer.rs`
- `src/app/operations/ui_rendering/list_bridge.rs`

Tasks:

1. Add tag lookup references to `GridViewContext`, `ItemSlotContext`, and `ListViewContext`.
2. In `grid_bridge.rs` and `item_slot_bridge.rs`, pass tag metadata from `ImageViewerApp` into the contexts.
3. Add `render_tag_badge()` in `badges.rs`.
4. Render grid tag badges at the top-left of the thumbnail/folder rect. Keep OneDrive sync badges at bottom-right.
5. For multiple tags, render up to three small dots or a short color strip. Do not allocate in the item render hot path.
6. In list view, draw a small color dot between the file icon and the name text, then shift the name text only when tags exist.
7. Update list hit-testing in `list_item_content_contains_pointer()` to use the shifted name origin when a tag dot is present.

Hot-path rules:

- Look up tags once per item using the path already in `FileEntry`.
- Return early when an item has no tags.
- Do not query SQLite from render code.
- Do not allocate textures for colored dots; use `Painter::circle_filled()`.

### Phase 6 - Sidebar Tags Section

Files:

- `src/ui/sidebar.rs`
- `src/ui/app/panels/mod.rs`
- `src/tabs/mod.rs`
- `src/app/operations/tabs.rs`

Tasks:

1. Add to `SidebarContext`:

```rust
pub tag_definitions: &'a FxHashMap<i64, FileTag>,
pub tag_counts: &'a FxHashMap<i64, usize>,
pub active_tag_filter: Option<i64>,
pub collapse_tags: bool,
```

2. Add `SidebarAction::FilterByTag(Option<i64>)` and `SidebarAction::ToggleTags`.
3. Render a Tags section at the top of `render_sidebar_drives()` before cloud roots, so it appears between Quick Access and Cloud Drives while staying inside the scrollable area.
4. Each tag row: colored dot, tag name, count badge, active highlight.
5. Include an `All tags` or `Clear tag filter` row when a filter is active.
6. Handle the new actions in `handle_sidebar_action()` by calling `set_tag_filter()` and toggling `collapse_tags`.

### Phase 7 - Context Menu Tag Assignment

Files:

- `src/application/context_menu.rs`
- `src/ui/context_menu.rs`
- `src/app/operations/context_menu.rs`
- `src/ui/app/menu_handler.rs`

Tasks:

1. Extend `ContextMenuItem` with minimal optional UI metadata:

```rust
pub is_checked: bool,
pub leading_color: Option<egui::Color32>,
```

2. Render `is_checked` and `leading_color` in `src/ui/context_menu.rs` before the item text.
3. In `populate_context_menu()`, add a `Tag` submenu only when all targets are taggable real files/folders.
4. Build tag submenu items sorted by `position, name`.
5. Mark a tag checked when all target paths already have that tag.
6. Use a temporary negative item ID plus `command_string = Some(format!("tag_toggle:{tag_id}"))`. Do not encode arbitrary `i64` tag IDs directly into the negative item ID.
7. Add a `Manage Tags...` item with a fixed negative ID.
8. In `handle_context_menu()`, find the selected item's `command_string` by ID and parse `tag_toggle:{id}` before falling through to the existing `match id`.
9. Call `toggle_tag_on_paths(&context_menu.target_paths, tag_id)` for toggle commands.
10. Open the tag manager for `Manage Tags...`.

Placement:

- Insert the Tag submenu after the OneDrive cloud items block and before Properties.
- Do not show it in Recycle Bin, This PC, empty-area context menus, drive-root context menus, or unsupported shell paths.

### Phase 8 - Tag Manager Modal

Files:

- `src/ui/components/tag_manager_modal.rs`
- `src/ui/components/mod.rs`
- `src/ui/app_impl.rs`

Tasks:

1. Add `pub mod tag_manager_modal;` in `components/mod.rs`.
2. Render the modal from `app_impl.rs` when `app.show_tag_manager` is true.
3. Implement create, rename, recolor, and delete flows.
4. Validate names: trim whitespace, reject empty names, rely on DB uniqueness for case-insensitive duplicates, and surface a notification on failure.
5. Deleting a tag with assignments should show a confirmation because it removes assignments.
6. Apply successful changes immediately to DB, memory caches, tag counts, and visible filtering.

Keep the modal self-contained. Do not add a large settings-page integration for V1.

### Phase 9 - Internationalization

Files:

- `locales/en.yml`
- `locales/pt-BR.yml`

Suggested keys:

```yaml
tags:
  section: "Tags"
  assign: "Tag"
  manage: "Manage Tags..."
  manager_title: "Manage Tags"
  add: "Add tag"
  name: "Tag name"
  color: "Color"
  delete: "Delete tag"
  delete_confirm: "Delete tag '%{name}'? This will remove it from %{count} item(s)."
  clear_filter: "Clear tag filter"
  all: "All tags"
  no_tags: "No tags"
  duplicate_name: "A tag with this name already exists."
  invalid_name: "Tag name cannot be empty."
  filter_active: "Tag: %{name}"
  color_red: "Red"
  color_orange: "Orange"
  color_yellow: "Yellow"
  color_green: "Green"
  color_blue: "Blue"
  color_purple: "Purple"
  color_gray: "Gray"
```

### Phase 10 - Cleanup And Maintenance

Files:

- `src/infrastructure/app_state_db/gc.rs`
- `src/app/init_workers/background_jobs.rs`
- `src/app/operations/message_handler/file_op_events.rs`

Tasks:

1. Add `garbage_collect_tag_assignments_incremental(max_candidates: usize) -> usize` using the same bounded random-sample pattern as `garbage_collect_covers_incremental()`.
2. Reuse `path_exists_fast()` and accessible-drive filtering from `gc.rs` to avoid deleting assignments from temporarily disconnected drives.
3. Call tag assignment GC from `spawn_incremental_gc_worker()` next to disk-cache and folder-cover GC.
4. On app delete completion, eagerly remove assignments for deleted paths from memory and DB.

## Expected File Changes

### New Files

| File | Purpose |
|---|---|
| `src/domain/file_tag.rs` | Domain model and color conversion |
| `src/infrastructure/app_state_db/file_tags.rs` | SQLite CRUD and assignment persistence |
| `src/app/operations/tag_ops.rs` | App-level tag operations and cache updates |
| `src/ui/components/tag_manager_modal.rs` | Tag CRUD modal |

### Existing Files To Modify

| Area | Files |
|---|---|
| Domain/DB | `src/domain/mod.rs`, `src/infrastructure/app_state_db/mod.rs`, `src/infrastructure/app_state_db/gc.rs` |
| Startup/state/preferences | `src/app/state/mod.rs`, `src/app/init.rs`, `src/app/init_preferences.rs`, `src/app/operations/preferences.rs` |
| Tabs/dual panel | `src/tabs/mod.rs`, `src/app/operations/tabs.rs`, `src/app/dual_panel.rs` |
| Operations | `src/app/operations/mod.rs`, `src/app/operations/message_handler/file_op_events.rs` |
| Filtering | `src/application/sorting.rs`, `src/application/sorting/filtering.rs`, `src/app/operations/folder_loading/view_updates.rs`, `src/app/operations/message_handler/thumbnail_rebuild.rs` |
| Grid/list rendering | `src/ui/components/item_slot/*`, `src/ui/views/grid_view/*`, `src/ui/views/list_view/*`, `src/app/operations/ui_rendering/*bridge.rs` |
| Sidebar | `src/ui/sidebar.rs`, `src/ui/app/panels/mod.rs` |
| Context menu | `src/application/context_menu.rs`, `src/ui/context_menu.rs`, `src/app/operations/context_menu.rs`, `src/ui/app/menu_handler.rs` |
| Modal/i18n | `src/ui/components/mod.rs`, `src/ui/app_impl.rs`, `locales/en.yml`, `locales/pt-BR.yml` |
| Maintenance | `src/app/init_workers/background_jobs.rs` |

## Risks And Mitigations

| Risk | Mitigation |
|---|---|
| Relying on SQLite cascade while foreign keys are disabled | Enable `PRAGMA foreign_keys = ON` per connection or explicitly delete assignments before deleting tags. Do both for safety if the pragma is added. |
| Path tags do not follow external moves/renames | Document V1 as path-based. Preserve tags only for app-initiated rename/move where source and destination are known. |
| Large assignment map clone during background rebuilds | Store assignments as `Arc<FxHashMap<...>>` and capture cheap snapshots in rebuild threads. |
| Render hot-path overhead | No DB calls, no texture allocation, early return for untagged items, use `Painter` primitives. |
| Tag filter bleeding between dual panels | Add `active_tag_filter` to `PanelSnapshot` and include it in `from_app`, `apply_to`, and `swap_with_app`. |
| Context menu cannot show checkmarks/colors today | Extend `ContextMenuItem` minimally with `is_checked` and `leading_color`. |
| Default tag localization | Seed stable names in DB because migrations run before locale is applied. UI color labels remain localized; users can rename default tags. |
| Deleted files on disconnected drives | GC must skip orphan deletion for inaccessible drives, matching existing cover/disk-cache GC behavior. |

## Verification Checklist

- `cargo fmt`
- `cargo check`
- Unit tests for `TagColor` DB conversion.
- Unit or integration test for AppStateDb tag CRUD if test DB helpers are available.
- Manual: assign/unassign one tag to one file.
- Manual: assign multiple tags to multiple selected items.
- Manual: grid badge, list dot, and sidebar counts update without restart.
- Manual: tag filter works with and without search query.
- Manual: tab switch and dual-panel switch preserve independent active filters.
- Manual: rename/move in app preserves tags where destination is known.
- Manual: delete removes assignments and updates counts.
- Manual: deleting a tag removes assignments and clears active filter if needed.
- Manual: Recycle Bin, This PC, drive roots, and empty-area context menus do not show tag assignment actions.

## Rejected Alternatives

1. NTFS Alternate Data Streams: rejected because tags are lost or unsupported on FAT/exFAT/network/cloud-backed paths.
2. Storing tags directly in `FileEntry`: rejected because `FileEntry` is rebuilt from filesystem scans and should not own persistent metadata.
3. Separate `tags.db`: rejected because `app_state.db` already handles lightweight app metadata with the right connection pattern.
4. Global search service integration for V1: rejected because local path/tag filtering is enough for current folder filtering.
5. GPU textures for tag dots: rejected because egui painter primitives are cheaper and avoid cache management.
