# Indexing Strategies

<cite>
**Referenced Files in This Document**
- [main.rs](file://crates/mtt-search-service/src/main.rs)
- [usn_journal.rs](file://crates/mtt-search-service/src/usn_journal.rs)
- [volume_indexers/mod.rs](file://crates/mtt-search-service/src/volume_indexers/mod.rs)
- [volume_indexers/usn.rs](file://crates/mtt-search-service/src/volume_indexers/usn.rs)
- [volume_indexers/non_usn.rs](file://crates/mtt-search-service/src/volume_indexers/non_usn.rs)
- [fs_walker.rs](file://crates/mtt-search-service/src/fs_walker.rs)
- [mft_reader.rs](file://crates/mtt-search-service/src/mft_reader.rs)
- [file_index.rs](file://crates/mtt-search-service/src/file_index.rs)
- [indexing_progress.rs](file://crates/mtt-search-service/src/indexing_progress.rs)
- [volume_indices.rs](file://crates/mtt-search-service/src/volume_indices.rs)
- [index_db/mod.rs](file://crates/mtt-search-service/src/index_db/mod.rs)
- [index_db/binary.rs](file://crates/mtt-search-service/src/index_db/binary.rs)
</cite>

## Table of Contents
1. [Introduction](#introduction)
2. [Project Structure](#project-structure)
3. [Core Components](#core-components)
4. [Architecture Overview](#architecture-overview)
5. [Detailed Component Analysis](#detailed-component-analysis)
6. [Dependency Analysis](#dependency-analysis)
7. [Performance Considerations](#performance-considerations)
8. [Troubleshooting Guide](#troubleshooting-guide)
9. [Conclusion](#conclusion)

## Introduction
This document explains the hybrid indexing strategies used by the search service to efficiently index local file systems across multiple drive types. It covers:
- USN (Update Sequence Number) journal-based indexing for NTFS and ReFS volumes, including real-time change tracking and incremental updates.
- Fallback full-tree scanning for FAT32, exFAT, and network drives that lack USN support.
- The file system walker that traverses directory structures and extracts metadata.
- The MFT (Master File Table) reader for NTFS volumes to gather comprehensive file information.
- Indexing progress tracking, volume discovery mechanisms, and concurrent indexing across multiple drives.
- Performance characteristics, memory usage patterns, and optimization strategies for large-scale indexing operations.

## Project Structure
The search service is organized into cohesive modules:
- Discovery and orchestration: main entrypoint spawns indexers for discovered volumes.
- Indexer strategies: USN-based and fallback non-USN indexers.
- Core indexing primitives: file system walker, MFT reader, in-memory index model, and persistence.
- Concurrency and progress: per-volume handles, shared progress tracker, and FTS state.

```mermaid
graph TB
subgraph "Orchestration"
MAIN["main.rs"]
DISC["usn_journal.rs<br/>discover_volumes()"]
VOL_IDX["volume_indices.rs"]
PROG["indexing_progress.rs"]
end
subgraph "Indexer Strategies"
USN_IDX["volume_indexers/usn.rs"]
NON_USN_IDX["volume_indexers/non_usn.rs"]
WALKER["fs_walker.rs"]
MFT["mft_reader.rs"]
end
subgraph "Core Data"
FIDX["file_index.rs"]
DB["index_db/mod.rs"]
BIN["index_db/binary.rs"]
end
MAIN --> DISC
MAIN --> VOL_IDX
MAIN --> PROG
DISC --> USN_IDX
DISC --> NON_USN_IDX
USN_IDX --> MFT
USN_IDX --> DB
USN_IDX --> FIDX
NON_USN_IDX --> WALKER
NON_USN_IDX --> DB
NON_USN_IDX --> FIDX
DB --> BIN
VOL_IDX --> USN_IDX
VOL_IDX --> NON_USN_IDX
```

**Diagram sources**
- [main.rs:190-307](file://crates/mtt-search-service/src/main.rs#L190-L307)
- [usn_journal.rs:80-138](file://crates/mtt-search-service/src/usn_journal.rs#L80-L138)
- [volume_indexers/mod.rs:10-27](file://crates/mtt-search-service/src/volume_indexers/mod.rs#L10-L27)
- [volume_indexers/usn.rs:39-714](file://crates/mtt-search-service/src/volume_indexers/usn.rs#L39-L714)
- [volume_indexers/non_usn.rs:35-237](file://crates/mtt-search-service/src/volume_indexers/non_usn.rs#L35-L237)
- [fs_walker.rs:24-136](file://crates/mtt-search-service/src/fs_walker.rs#L24-L136)
- [mft_reader.rs:1214-1491](file://crates/mtt-search-service/src/mft_reader.rs#L1214-L1491)
- [file_index.rs:58-104](file://crates/mtt-search-service/src/file_index.rs#L58-L104)
- [index_db/mod.rs:282-385](file://crates/mtt-search-service/src/index_db/mod.rs#L282-L385)
- [index_db/binary.rs:1-32](file://crates/mtt-search-service/src/index_db/binary.rs#L1-L32)

**Section sources**
- [main.rs:190-307](file://crates/mtt-search-service/src/main.rs#L190-L307)
- [usn_journal.rs:80-138](file://crates/mtt-search-service/src/usn_journal.rs#L80-L138)
- [volume_indexers/mod.rs:10-27](file://crates/mtt-search-service/src/volume_indexers/mod.rs#L10-L27)

## Core Components
- Volume discovery and spawning: discovers volumes, determines USN capability, and spawns per-drive indexers concurrently.
- USN-based indexer: loads cached state, opens journal, enumerates incrementally, falls back to full MFT scan when needed, persists snapshots, and runs continuous incremental updates.
- Fallback indexer: performs periodic full scans with adaptive backoff, optionally using change monitoring for responsiveness, persists to DB, and rebuilds FTS.
- File system walker: iterative directory traversal for non-USN volumes, skipping reparse points to avoid cycles.
- MFT reader: bulk sequential read of NTFS $MFT to extract names, parents, sizes, hardlinks, and reparse flags in one pass.
- In-memory index model: compact records, reverse children index, name arena, and helpers for folder size computation.
- Persistence: SQLite-backed storage plus a fast binary format for cached volumes; FTS5 integration and integrity protection.
- Concurrency and progress: per-volume handles with independent locks, shared progress tracker, and FTS readiness state.

**Section sources**
- [main.rs:240-387](file://crates/mtt-search-service/src/main.rs#L240-L387)
- [volume_indexers/usn.rs:39-714](file://crates/mtt-search-service/src/volume_indexers/usn.rs#L39-L714)
- [volume_indexers/non_usn.rs:35-237](file://crates/mtt-search-service/src/volume_indexers/non_usn.rs#L35-L237)
- [fs_walker.rs:24-136](file://crates/mtt-search-service/src/fs_walker.rs#L24-L136)
- [mft_reader.rs:1214-1491](file://crates/mtt-search-service/src/mft_reader.rs#L1214-L1491)
- [file_index.rs:58-104](file://crates/mtt-search-service/src/file_index.rs#L58-L104)
- [index_db/mod.rs:282-385](file://crates/mtt-search-service/src/index_db/mod.rs#L282-L385)

## Architecture Overview
The indexing architecture combines real-time change tracking with robust fallbacks and concurrency.

```mermaid
sequenceDiagram
participant Orchestrator as "main.rs"
participant Discover as "usn_journal.rs"
participant USN as "volume_indexers/usn.rs"
participant NonUSN as "volume_indexers/non_usn.rs"
participant Walker as "fs_walker.rs"
participant MFT as "mft_reader.rs"
participant DB as "index_db/mod.rs"
Orchestrator->>Discover : discover_volumes()
Discover-->>Orchestrator : [DiscoveredVolume...]
Orchestrator->>USN : spawn_indexer(usn_supported=true)
Orchestrator->>NonUSN : spawn_indexer(usn_supported=false)
USN->>DB : load_volume_state()/load_into_index()
alt cached state available
USN->>USN : read_usn_changes() catch-up
else no or stale cache
USN->>MFT : read_mft_bulk()
MFT-->>USN : VolumeIndex
USN->>DB : save_volume_state_snapshot()
end
USN->>USN : incremental loop (read_usn_buffer -> apply -> persist snapshots)
NonUSN->>Walker : scan_volume()
Walker-->>NonUSN : VolumeIndex
NonUSN->>DB : save_volume()
```

**Diagram sources**
- [main.rs:240-387](file://crates/mtt-search-service/src/main.rs#L240-L387)
- [usn_journal.rs:80-138](file://crates/mtt-search-service/src/usn_journal.rs#L80-L138)
- [volume_indexers/usn.rs:39-714](file://crates/mtt-search-service/src/volume_indexers/usn.rs#L39-L714)
- [volume_indexers/non_usn.rs:35-237](file://crates/mtt-search-service/src/volume_indexers/non_usn.rs#L35-L237)
- [fs_walker.rs:24-136](file://crates/mtt-search-service/src/fs_walker.rs#L24-L136)
- [mft_reader.rs:1214-1491](file://crates/mtt-search-service/src/mft_reader.rs#L1214-L1491)
- [index_db/mod.rs:506-598](file://crates/mtt-search-service/src/index_db/mod.rs#L506-L598)

## Detailed Component Analysis

### USN-Based Indexing for NTFS and ReFS
The USN indexer provides near real-time change tracking and efficient incremental updates:
- State loading: attempts to load a cached binary index; if absent or incomplete, loads from SQLite and injects into the in-memory index.
- Journal operations: opens the volume, queries journal info, and reads USN buffers to apply incremental changes.
- Catch-up logic: if cached state is compatible, it catches up from the last USN; on failure, it falls back to a full MFT scan.
- Full scan path: bulk MFT read to enumerate all files, compact arena, and mark sizes as loaded.
- Incremental loop: periodically reads the USN buffer, applies changes under bounded contention, refreshes sizes for changed files, persists snapshots every five minutes, and prunes stale metadata.
- Background size extraction: if sizes were not loaded from cache, a background thread performs a bulk MFT read to fill sizes.

```mermaid
flowchart TD
Start(["Start USN Indexer"]) --> LoadState["Load cached state (binary/SQLite)"]
LoadState --> HasCache{"Compatible cache?"}
HasCache --> |Yes| CatchUp["Catch-up from last USN"]
CatchUp --> CatchUpOK{"Catch-up succeeded?"}
CatchUpOK --> |Yes| Ready["Index Ready"]
CatchUpOK --> |No| FullScan["Full MFT scan"]
HasCache --> |No| FullScan
FullScan --> Ready
Ready --> IncLoop["Incremental loop"]
IncLoop --> ReadBuffer["Read USN buffer"]
ReadBuffer --> Apply["Apply changes under bounded contention"]
Apply --> SizeRefresh["Drain pending_size_refresh and refresh sizes"]
SizeRefresh --> Persist["Persist snapshot every 5 min"]
Persist --> IncLoop
```

**Diagram sources**
- [volume_indexers/usn.rs:39-714](file://crates/mtt-search-service/src/volume_indexers/usn.rs#L39-L714)
- [mft_reader.rs:1214-1491](file://crates/mtt-search-service/src/mft_reader.rs#L1214-L1491)
- [index_db/mod.rs:506-598](file://crates/mtt-search-service/src/index_db/mod.rs#L506-L598)

**Section sources**
- [volume_indexers/usn.rs:39-714](file://crates/mtt-search-service/src/volume_indexers/usn.rs#L39-L714)
- [usn_journal.rs:170-314](file://crates/mtt-search-service/src/usn_journal.rs#L170-L314)
- [mft_reader.rs:1214-1491](file://crates/mtt-search-service/src/mft_reader.rs#L1214-L1491)
- [index_db/binary.rs:1-32](file://crates/mtt-search-service/src/index_db/binary.rs#L1-L32)

### Fallback Full-Tree Scanning for FAT/exFAT/Network Drives
For file systems without USN support, the fallback indexer performs periodic full scans:
- Adaptive backoff: increases wait intervals when no changes are detected; resets on detected changes or external triggers.
- Change monitoring: optional ReadDirectoryChangesW-based monitoring for responsive wake-ups on supported file systems.
- Scan and persist: iteratively walks directories, inserts records into the in-memory index, persists to DB, and rebuilds FTS in the background.
- Periodic scheduling: loops with calculated intervals to balance responsiveness and I/O.

```mermaid
flowchart TD
Start(["Start Fallback Indexer"]) --> LoadCached["Optionally load cached DB index"]
LoadCached --> Loop["Loop with adaptive backoff"]
Loop --> Scan["Full-tree scan_volume()"]
Scan --> Persist["Save to DB and clear pending"]
Persist --> MaybeFTS["Background FTS rebuild"]
MaybeFTS --> Wait["Wait for change or timeout"]
Wait --> Decision{"Shutdown/Change?"}
Decision --> |Shutdown| End(["Stop"])
Decision --> |Change| Loop
```

**Diagram sources**
- [volume_indexers/non_usn.rs:35-237](file://crates/mtt-search-service/src/volume_indexers/non_usn.rs#L35-L237)
- [fs_walker.rs:24-136](file://crates/mtt-search-service/src/fs_walker.rs#L24-L136)
- [index_db/mod.rs:506-598](file://crates/mtt-search-service/src/index_db/mod.rs#L506-L598)

**Section sources**
- [volume_indexers/non_usn.rs:35-237](file://crates/mtt-search-service/src/volume_indexers/non_usn.rs#L35-L237)
- [fs_walker.rs:24-136](file://crates/mtt-search-service/src/fs_walker.rs#L24-L136)

### File System Walker Implementation
The walker performs an iterative directory traversal:
- Uses a queue to avoid recursion.
- Skips reparse points to prevent cycles.
- Inserts records into the in-memory index and reports progress.
- Respects shutdown signals.

```mermaid
flowchart TD
Start(["scan_volume()"]) --> Init["Initialize queue with root and parent ref"]
Init --> WhileQueue{"Queue not empty?"}
WhileQueue --> |Yes| Dequeue["Pop (dir_path, parent_ref)"]
Dequeue --> Iterate["Iterate entries with read_dir()"]
Iterate --> Insert["Insert record (name, parent_ref, is_dir, is_reparse)"]
Insert --> Enqueue{"Is directory and not reparse?"}
Enqueue --> |Yes| Push["Push into queue"]
Enqueue --> |No| Continue["Continue"]
Continue --> WhileQueue
WhileQueue --> |No| Done(["Return stats"])
```

**Diagram sources**
- [fs_walker.rs:24-136](file://crates/mtt-search-service/src/fs_walker.rs#L24-L136)

**Section sources**
- [fs_walker.rs:24-136](file://crates/mtt-search-service/src/fs_walker.rs#L24-L136)

### MFT Reader for NTFS Volumes
The MFT reader performs a single sequential pass over the NTFS $MFT:
- Queries MFT geometry and data runs from record 0.
- Reads aligned chunks, applies fixups, parses records, and populates the index.
- Extracts sizes from $DATA attributes, resolves external sizes via $ATTRIBUTE_LIST, and repairs zero-sized entries.
- Applies extension record sizes to base records and reconstructs hardlink parent edges.

```mermaid
flowchart TD
Start(["read_mft_bulk()"]) --> Geo["Query MFT geometry"]
Geo --> Runs["Get MFT data runs from record 0"]
Runs --> ReadLoop["Sequential read with aligned chunks"]
ReadLoop --> Fixup["Apply NTFS fixups"]
Fixup --> Parse["Parse records and extract names/parents/sizes"]
Parse --> ExtSizes["Collect extension sizes and parents"]
ExtSizes --> Merge["Merge extension sizes into base records"]
Merge --> Repair["Repair zero-sized entries"]
Repair --> Done(["Return VolumeIndex"])
```

**Diagram sources**
- [mft_reader.rs:1214-1491](file://crates/mtt-search-service/src/mft_reader.rs#L1214-L1491)

**Section sources**
- [mft_reader.rs:1214-1491](file://crates/mtt-search-service/src/mft_reader.rs#L1214-L1491)

### Index Model and Reverse Children Index
The in-memory index model supports efficient traversal and folder size computation:
- Compact records with name references into a contiguous arena.
- Reverse children index enabling O(subtree) traversal for folder size calculations.
- Helpers for hardlink parent edges, reparse point tracking, and arena compaction.

```mermaid
classDiagram
class VolumeIndex {
+char drive_letter
+HashMap<u64, FileRecord> records
+HashMap<u64, Vec<u64>> children
+NameArena names
+i64 last_usn
+u64 journal_id
+IndexState state
+bool sizes_loaded
+HashSet<u64> pending_additions
+HashSet<u64> pending_removals
+HashMap<u64, Instant> dir_modified_at
+HashSet<u64> pending_size_refresh
+HashMap<u64, Vec<u64>> hardlink_parents
+HashSet<u64> reparse_points
+bool hardlink_data_complete
+bool reparse_data_complete
+insert_record(...)
+remove_record(u64)
+move_record(...)
+clear()
+clear_pending()
+prune_old_modifications(Duration)
+compact_arena()
+rebuild_children()
+folder_tree_summary(u64) (u64,u64,u64)
+memory_usage() (usize, usize, usize)
}
class FileRecord {
+u64 parent_ref
+u64 size
+u32 name_offset
+u16 name_len
+bool is_dir
+u8 _pad
+name_ref() NameRef
}
VolumeIndex --> FileRecord : "stores"
```

**Diagram sources**
- [file_index.rs:58-104](file://crates/mtt-search-service/src/file_index.rs#L58-L104)
- [file_index.rs:18-47](file://crates/mtt-search-service/src/file_index.rs#L18-L47)

**Section sources**
- [file_index.rs:58-104](file://crates/mtt-search-service/src/file_index.rs#L58-L104)
- [file_index.rs:38-47](file://crates/mtt-search-service/src/file_index.rs#L38-L47)

### Volume Discovery and Concurrent Indexing
The orchestrator discovers volumes, spawns indexers, and maintains shared state:
- Discovers volumes and categorizes by USN support.
- Spawns per-volume indexers concurrently with independent lifecycle.
- Maintains shared progress tracker and FTS readiness state.

```mermaid
sequenceDiagram
participant Main as "main.rs"
participant Disc as "usn_journal.rs"
participant Spawn as "main.rs"
participant USN as "volume_indexers/usn.rs"
participant NonUSN as "volume_indexers/non_usn.rs"
Main->>Disc : discover_volumes()
Disc-->>Main : [DiscoveredVolume...]
loop For each volume
Main->>Spawn : spawn_volume_indexer()
alt USN supported
Spawn->>USN : index_volume()
else
Spawn->>NonUSN : index_non_ntfs_volume()
end
end
```

**Diagram sources**
- [main.rs:240-387](file://crates/mtt-search-service/src/main.rs#L240-L387)
- [usn_journal.rs:80-138](file://crates/mtt-search-service/src/usn_journal.rs#L80-L138)
- [volume_indexers/mod.rs:39-7](file://crates/mtt-search-service/src/volume_indexers/mod.rs#L39-L7)

**Section sources**
- [main.rs:240-387](file://crates/mtt-search-service/src/main.rs#L240-L387)
- [usn_journal.rs:80-138](file://crates/mtt-search-service/src/usn_journal.rs#L80-L138)

## Dependency Analysis
Key dependencies and coupling:
- Indexers depend on the in-memory index model and persistence layer.
- USN indexer depends on the USN journal module for journal operations and change parsing.
- Fallback indexer depends on the file system walker and DB persistence.
- Concurrency is achieved via per-volume handles; the outer lock manages membership while inner locks guard individual indices.
- Progress tracking and FTS state are shared across indexers and the IPC server.

```mermaid
graph TB
USN["volume_indexers/usn.rs"] --> JRN["usn_journal.rs"]
USN --> MFT["mft_reader.rs"]
USN --> DB["index_db/mod.rs"]
USN --> FIDX["file_index.rs"]
NON["volume_indexers/non_usn.rs"] --> WALK["fs_walker.rs"]
NON --> DB
NON --> FIDX
MAIN["main.rs"] --> DISC["usn_journal.rs"]
MAIN --> VOLIDX["volume_indices.rs"]
MAIN --> PROG["indexing_progress.rs"]
DB --> BIN["index_db/binary.rs"]
```

**Diagram sources**
- [volume_indexers/usn.rs:39-714](file://crates/mtt-search-service/src/volume_indexers/usn.rs#L39-L714)
- [volume_indexers/non_usn.rs:35-237](file://crates/mtt-search-service/src/volume_indexers/non_usn.rs#L35-L237)
- [usn_journal.rs:170-314](file://crates/mtt-search-service/src/usn_journal.rs#L170-L314)
- [mft_reader.rs:1214-1491](file://crates/mtt-search-service/src/mft_reader.rs#L1214-L1491)
- [index_db/mod.rs:282-385](file://crates/mtt-search-service/src/index_db/mod.rs#L282-L385)
- [index_db/binary.rs:1-32](file://crates/mtt-search-service/src/index_db/binary.rs#L1-L32)
- [main.rs:240-387](file://crates/mtt-search-service/src/main.rs#L240-L387)

**Section sources**
- [volume_indices.rs:43-58](file://crates/mtt-search-service/src/volume_indices.rs#L43-L58)
- [indexing_progress.rs:16-49](file://crates/mtt-search-service/src/indexing_progress.rs#L16-L49)

## Performance Considerations
- Memory usage:
  - Compact records (24 bytes) and a contiguous name arena minimize overhead.
  - Estimated memory usage reported via memory_usage() includes arena usage and hashmap capacity estimates.
  - Arena compaction reduces dead space after bulk operations and incremental overwrites.
- I/O efficiency:
  - USN-based catch-up avoids full scans when cache is valid.
  - Bulk MFT read performs a single sequential pass over the $MFT, reducing per-file IO overhead.
  - Fallback indexer uses adaptive backoff to reduce idle I/O.
- Concurrency:
  - Independent per-volume locks enable concurrent indexing across multiple drives without cross-volume contention.
  - Incremental writes use bounded retry and fallback timeouts to avoid read starvation.
- Persistence:
  - Binary cache provides fast restart for USN volumes.
  - SQLite persists records and hardlink parents; FTS5 is rebuilt only when necessary.
- Search performance:
  - Lowercased name arena and SIMD-based substring search improve lookup speed.
  - Reverse children index enables O(subtree) folder size computations.

[No sources needed since this section provides general guidance]

## Troubleshooting Guide
Common issues and diagnostics:
- USN journal disabled or inaccessible:
  - Symptoms: failures when opening or querying the journal.
  - Resolution: ensure the USN journal is enabled and the process has appropriate privileges.
- Journal wraparound or expired USN:
  - Symptoms: catch-up fails with “journal entries expired”.
  - Resolution: trigger a full MFT scan to rebuild the index.
- Name arena full:
  - Symptoms: scans stop early with arena full messages.
  - Resolution: adjust indexing limits or reduce scope; consider re-running with larger capacity.
- Lock contention during incremental updates:
  - Symptoms: periodic skipped cycles or retries.
  - Resolution: verify bounded contention logs and ensure adequate CPU resources; consider reducing concurrent writes.
- Fallback scan thrashing:
  - Symptoms: frequent scans despite no changes.
  - Resolution: verify change monitoring availability and adjust cadence; confirm adaptive backoff is functioning.

**Section sources**
- [usn_journal.rs:170-314](file://crates/mtt-search-service/src/usn_journal.rs#L170-L314)
- [volume_indexers/usn.rs:476-620](file://crates/mtt-search-service/src/volume_indexers/usn.rs#L476-L620)
- [volume_indexers/non_usn.rs:214-234](file://crates/mtt-search-service/src/volume_indexers/non_usn.rs#L214-L234)

## Conclusion
The search service employs a hybrid indexing strategy tailored to each file system:
- NTFS and ReFS volumes benefit from USN-based real-time change tracking with robust fallbacks and efficient bulk MFT enumeration.
- FAT32, exFAT, and network drives rely on adaptive full-tree scanning with change monitoring and periodic persistence.
- The in-memory index model, reverse children index, and SIMD search deliver strong performance characteristics.
- Concurrency and persistence mechanisms ensure scalable, resilient indexing across heterogeneous environments.

[No sources needed since this section summarizes without analyzing specific files]