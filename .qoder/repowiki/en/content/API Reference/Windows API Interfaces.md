# Windows API Interfaces

<cite>
**Referenced Files in This Document**
- [mod.rs](file://src/infrastructure/windows/mod.rs)
- [shell_operations.rs](file://src/infrastructure/windows/shell_operations.rs)
- [context_menu.rs](file://src/infrastructure/windows/shell_operations/context_menu.rs)
- [file_op.rs](file://src/infrastructure/windows/shell_operations/file_op.rs)
- [shfile_ops.rs](file://src/infrastructure/windows/shell_operations/shfile_ops.rs)
- [file_system.rs](file://src/infrastructure/windows/file_system.rs)
- [drives.rs](file://src/infrastructure/windows/drives.rs)
- [system_info.rs](file://src/infrastructure/windows/system_info.rs)
- [codec_registry.rs](file://src/infrastructure/windows/codec_registry.rs)
- [known_codecs.rs](file://src/infrastructure/windows/codec_registry/known_codecs.rs)
- [mf_queries.rs](file://src/infrastructure/windows/codec_registry/mf_queries.rs)
- [registry_queries.rs](file://src/infrastructure/windows/codec_registry/registry_queries.rs)
- [media_foundation.rs](file://src/infrastructure/windows/media_foundation.rs)
- [com_scope.rs](file://src/infrastructure/windows/com_scope.rs)
- [main.rs](file://crates/mtt-search-service/src/main.rs)
- [Cargo.toml](file://crates/mtt-search-service/Cargo.toml)
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
This document describes the Windows API integration points implemented in the project, focusing on:
- Shell operations: file operations (copy, move, delete, rename), context menu integration, and shell namespace manipulation
- File system operations: NTFS-specific features, USN journal integration, and drive enumeration
- Windows service integration for the search service: installation, startup parameters, and inter-process communication
- Windows-specific utilities: codec registry queries, Media Foundation integration, and system information retrieval
- COM interface usage patterns, handle management, and error handling strategies specific to Windows API calls

## Project Structure
The Windows integration is organized under a dedicated module tree with clear separation of concerns:
- Shell operations: context menus, IFileOperation-based file operations, and SHFileOperation-based operations
- File system utilities: attribute checks, Windows path detection, and low-level Windows APIs
- Drive and volume utilities: volume label rename, drive enumeration, filesystem detection, and USN capability checks
- System information: drive type detection and memory usage
- Codec registry: dynamic codec name resolution via Media Foundation and registry
- Media Foundation: video metadata extraction as a robust fallback
- COM scope: RAII wrappers for COM initialization
- Search service: Windows service integration, IPC server, and volume indexing

```mermaid
graph TB
subgraph "Windows Module"
A["mod.rs"]
B["shell_operations.rs"]
C["file_system.rs"]
D["drives.rs"]
E["system_info.rs"]
F["codec_registry.rs"]
G["media_foundation.rs"]
H["com_scope.rs"]
end
subgraph "Shell Ops"
B1["context_menu.rs"]
B2["file_op.rs"]
B3["shfile_ops.rs"]
end
subgraph "Search Service"
S1["main.rs"]
S2["Cargo.toml"]
end
A --> B
A --> C
A --> D
A --> E
A --> F
A --> G
A --> H
B --> B1
B --> B2
B --> B3
S1 --> D
S1 --> G
S2 --> S1
```

**Diagram sources**
- [mod.rs:1-60](file://src/infrastructure/windows/mod.rs#L1-L60)
- [shell_operations.rs:1-15](file://src/infrastructure/windows/shell_operations.rs#L1-L15)
- [context_menu.rs:1-194](file://src/infrastructure/windows/shell_operations/context_menu.rs#L1-L194)
- [file_op.rs:1-245](file://src/infrastructure/windows/shell_operations/file_op.rs#L1-L245)
- [shfile_ops.rs:1-242](file://src/infrastructure/windows/shell_operations/shfile_ops.rs#L1-L242)
- [file_system.rs:1-43](file://src/infrastructure/windows/file_system.rs#L1-L43)
- [drives.rs:1-550](file://src/infrastructure/windows/drives.rs#L1-L550)
- [system_info.rs:1-100](file://src/infrastructure/windows/system_info.rs#L1-L100)
- [codec_registry.rs:1-444](file://src/infrastructure/windows/codec_registry.rs#L1-L444)
- [media_foundation.rs:1-439](file://src/infrastructure/windows/media_foundation.rs#L1-L439)
- [com_scope.rs:1-41](file://src/infrastructure/windows/com_scope.rs#L1-L41)
- [main.rs:1-389](file://crates/mtt-search-service/src/main.rs#L1-L389)
- [Cargo.toml:1-33](file://crates/mtt-search-service/Cargo.toml#L1-L33)

**Section sources**
- [mod.rs:1-60](file://src/infrastructure/windows/mod.rs#L1-L60)
- [shell_operations.rs:1-15](file://src/infrastructure/windows/shell_operations.rs#L1-L15)
- [drives.rs:1-550](file://src/infrastructure/windows/drives.rs#L1-L550)
- [main.rs:1-389](file://crates/mtt-search-service/src/main.rs#L1-L389)

## Core Components
- Shell operations
  - Context menu integration via COM-based IShellFolder and IContextMenu
  - Robust file operations using IFileOperation with fallback to SHFileOperation
  - Single and batch copy/move/delete/rename operations with progress dialogs and undo support
- File system utilities
  - Attribute checks, directory/file detection, and Windows system path suppression
- Drive and volume utilities
  - Volume label rename with elevation handling and helper process orchestration
  - Drive enumeration, volume information retrieval, and filesystem capability checks
- System information
  - Drive type classification and current process memory usage
- Codec registry
  - Dynamic codec name resolution using Media Foundation, registry, and known mappings
- Media Foundation
  - Video metadata extraction via IMFSourceReader with COM and MF guards
- COM scope
  - RAII wrappers for COM initialization and uninitialization
- Search service
  - Windows service control, console mode, and IPC server with shared state

**Section sources**
- [context_menu.rs:1-194](file://src/infrastructure/windows/shell_operations/context_menu.rs#L1-L194)
- [file_op.rs:1-245](file://src/infrastructure/windows/shell_operations/file_op.rs#L1-L245)
- [shfile_ops.rs:1-242](file://src/infrastructure/windows/shell_operations/shfile_ops.rs#L1-L242)
- [file_system.rs:1-43](file://src/infrastructure/windows/file_system.rs#L1-L43)
- [drives.rs:1-550](file://src/infrastructure/windows/drives.rs#L1-L550)
- [system_info.rs:1-100](file://src/infrastructure/windows/system_info.rs#L1-L100)
- [codec_registry.rs:1-444](file://src/infrastructure/windows/codec_registry.rs#L1-L444)
- [media_foundation.rs:1-439](file://src/infrastructure/windows/media_foundation.rs#L1-L439)
- [com_scope.rs:1-41](file://src/infrastructure/windows/com_scope.rs#L1-L41)
- [main.rs:112-307](file://crates/mtt-search-service/src/main.rs#L112-L307)

## Architecture Overview
The Windows integration follows a layered architecture:
- Low-level Windows API wrappers (attributes, drives, system info)
- Shell operation abstractions (COM-based and SHFileOperation-based)
- Utility modules (codec registry, Media Foundation)
- Service layer (search service) orchestrating indexing and IPC

```mermaid
graph TB
subgraph "Low-Level Wrappers"
FS["file_system.rs"]
DV["drives.rs"]
SI["system_info.rs"]
end
subgraph "Shell Abstractions"
CM["context_menu.rs"]
IF["file_op.rs"]
SF["shfile_ops.rs"]
end
subgraph "Utilities"
CR["codec_registry.rs"]
MF["media_foundation.rs"]
CS["com_scope.rs"]
end
subgraph "Service"
MS["main.rs"]
end
FS --> CM
DV --> CM
SI --> CM
CM --> IF
IF --> SF
CR --> MF
CS --> CM
CS --> IF
CS --> MF
MS --> DV
MS --> MF
```

**Diagram sources**
- [file_system.rs:1-43](file://src/infrastructure/windows/file_system.rs#L1-L43)
- [drives.rs:1-550](file://src/infrastructure/windows/drives.rs#L1-L550)
- [system_info.rs:1-100](file://src/infrastructure/windows/system_info.rs#L1-L100)
- [context_menu.rs:1-194](file://src/infrastructure/windows/shell_operations/context_menu.rs#L1-L194)
- [file_op.rs:1-245](file://src/infrastructure/windows/shell_operations/file_op.rs#L1-L245)
- [shfile_ops.rs:1-242](file://src/infrastructure/windows/shell_operations/shfile_ops.rs#L1-L242)
- [codec_registry.rs:1-444](file://src/infrastructure/windows/codec_registry.rs#L1-L444)
- [media_foundation.rs:1-439](file://src/infrastructure/windows/media_foundation.rs#L1-L439)
- [com_scope.rs:1-41](file://src/infrastructure/windows/com_scope.rs#L1-L41)
- [main.rs:190-307](file://crates/mtt-search-service/src/main.rs#L190-L307)

## Detailed Component Analysis

### Shell Operations: Context Menu Integration
This component integrates with the native Windows shell to show context menus and invoke commands. It uses COM initialization, PIDL management, and IContextMenu to populate and execute shell commands.

```mermaid
sequenceDiagram
participant UI as "Caller"
participant CM as "show_shell_context_menu"
participant COM as "ComGuard"
participant SH as "Shell APIs"
participant Ctx as "IContextMenu"
UI->>CM : "Invoke with hwnd, path, screen_x, screen_y"
CM->>COM : "Initialize COM (STA)"
COM-->>CM : "OK or changed mode"
CM->>SH : "SHParseDisplayName()"
SH-->>CM : "PIDL"
CM->>SH : "SHBindToParent()"
SH-->>CM : "IShellFolder + child PIDL"
CM->>Ctx : "GetUIObjectOf(hwnd, items)"
Ctx-->>CM : "IContextMenu"
CM->>Ctx : "QueryContextMenu()"
CM->>SH : "TrackPopupMenuEx()"
alt "Command selected"
CM->>Ctx : "InvokeCommand(CMINVOKECOMMANDINFOEX)"
Ctx-->>CM : "Success"
else "Cancelled"
CM-->>UI : "was_cancelled=true"
end
CM-->>UI : "ContextMenuResult"
```

**Diagram sources**
- [context_menu.rs:75-193](file://src/infrastructure/windows/shell_operations/context_menu.rs#L75-L193)

**Section sources**
- [context_menu.rs:1-194](file://src/infrastructure/windows/shell_operations/context_menu.rs#L1-L194)

### Shell Operations: IFileOperation and SHFileOperation
This component provides robust file operations with fallbacks:
- IFileOperation-based copy/move supporting virtual paths and progress dialogs
- SHFileOperation-based single/batch operations with undo support and confirmation dialogs

```mermaid
sequenceDiagram
participant UI as "Caller"
participant FO as "copy_item_with_file_op"
participant COM as "FileOpComGuard"
participant SH as "Shell APIs"
participant OP as "IFileOperation"
UI->>FO : "Copy item with hwnd"
FO->>COM : "Initialize COM (STA)"
COM-->>FO : "Initialized"
FO->>SH : "CoCreateInstance(FileOperation)"
SH-->>FO : "IFileOperation"
FO->>SH : "SHCreateItemFromParsingName(src)"
SH-->>FO : "IShellItem src"
FO->>SH : "SHCreateItemFromParsingName(dest)"
SH-->>FO : "IShellItem dest"
FO->>OP : "CopyItem(src, dest)"
OP-->>FO : "OK"
FO->>OP : "PerformOperations()"
OP-->>FO : "OK or error"
FO-->>UI : "bool success"
```

**Diagram sources**
- [file_op.rs:31-71](file://src/infrastructure/windows/shell_operations/file_op.rs#L31-L71)

**Section sources**
- [file_op.rs:1-245](file://src/infrastructure/windows/shell_operations/file_op.rs#L1-L245)
- [shfile_ops.rs:1-242](file://src/infrastructure/windows/shell_operations/shfile_ops.rs#L1-L242)

### File System Utilities
Provides low-level file attribute checks and Windows system path detection.

```mermaid
flowchart TD
Start(["Entry"]) --> Attrs["GetFileAttributesW(path)"]
Attrs --> Valid{"INVALID or success?"}
Valid --> |INVALID| NotExists["Return false"]
Valid --> DirCheck{"DIRECTORY bit set?"}
DirCheck --> |Yes| IsDir["Return true (directory)"]
DirCheck --> |No| IsFile["Return true (file)"]
NotExists --> End(["Exit"])
IsDir --> End
IsFile --> End
```

**Diagram sources**
- [file_system.rs:8-31](file://src/infrastructure/windows/file_system.rs#L8-L31)

**Section sources**
- [file_system.rs:1-43](file://src/infrastructure/windows/file_system.rs#L1-L43)

### Drive and Volume Utilities
Handles volume label rename with elevation, drive enumeration, and filesystem capability checks.

```mermaid
sequenceDiagram
participant UI as "Caller"
participant DV as "rename_volume_label"
participant RAW as "set_volume_label_raw"
participant SE as "ShellExecuteExW"
participant WAIT as "WaitForSingleObject"
UI->>DV : "drive_path, new_label, hwnd"
DV->>RAW : "Try SetVolumeLabelW"
alt "Access Denied"
DV->>SE : "Launch elevated helper"
SE-->>DV : "Process handle"
DV->>WAIT : "Wait 30s"
WAIT-->>DV : "Exit code"
DV-->>UI : "RenamedElevated or error"
else "Success"
DV-->>UI : "Renamed"
end
```

**Diagram sources**
- [drives.rs:278-300](file://src/infrastructure/windows/drives.rs#L278-L300)
- [drives.rs:193-276](file://src/infrastructure/windows/drives.rs#L193-L276)

**Section sources**
- [drives.rs:1-550](file://src/infrastructure/windows/drives.rs#L1-L550)

### System Information
Provides drive type classification and current process memory usage.

```mermaid
flowchart TD
A["detect_drive_type(path)"] --> B["Ensure trailing backslash"]
B --> C["Encode path to wide string"]
C --> D["GetDriveTypeW(PCWSTR)"]
D --> E["Map to DriveType enum"]
E --> F["Return DriveType"]
G["get_ram_usage()"] --> H["GetCurrentProcess()"]
H --> I["K32GetProcessMemoryInfo()"]
I --> J{"Success?"}
J --> |Yes| K["Return WorkingSetSize"]
J --> |No| L["Return 0"]
```

**Diagram sources**
- [system_info.rs:64-81](file://src/infrastructure/windows/system_info.rs#L64-L81)
- [system_info.rs:84-99](file://src/infrastructure/windows/system_info.rs#L84-L99)

**Section sources**
- [system_info.rs:1-100](file://src/infrastructure/windows/system_info.rs#L1-L100)

### Codec Registry Integration
Resolves codec GUIDs to human-readable names using multiple strategies with caching.

```mermaid
flowchart TD
Start(["resolve_codec_guid(guid_str)"]) --> Cache["Check LRU cache"]
Cache --> |Hit| ReturnCache["Return cached name"]
Cache --> |Miss| Normalize["Normalize partial hex to GUID"]
Normalize --> Parse["Parse GUID string"]
Parse --> Known["check_known_codec(guid)"]
Known --> |Found| CachePutKnown["Cache and return"]
Known --> |Not Found| MF["query_mf_codec_name(guid)"]
MF --> |Found| CachePutMF["Cache and return"]
MF --> |Not Found| REG["query_registry_friendly_name(guid)"]
REG --> |Found| CachePutREG["Cache and return"]
REG --> |Not Found| Tag["query_waveformat_tag(guid.data1)"]
Tag --> |Found| CachePutTag["Cache and return"]
Tag --> |Not Found| Fallback["Microsoft codec name or FourCC decode"]
Fallback --> CachePutFall["Cache and return"]
```

**Diagram sources**
- [codec_registry.rs:57-167](file://src/infrastructure/windows/codec_registry.rs#L57-L167)
- [known_codecs.rs](file://src/infrastructure/windows/codec_registry/known_codecs.rs)
- [mf_queries.rs](file://src/infrastructure/windows/codec_registry/mf_queries.rs)
- [registry_queries.rs](file://src/infrastructure/windows/codec_registry/registry_queries.rs)

**Section sources**
- [codec_registry.rs:1-444](file://src/infrastructure/windows/codec_registry.rs#L1-L444)

### Media Foundation Integration
Extracts video metadata using IMFSourceReader with COM and MediaFoundation guards.

```mermaid
sequenceDiagram
participant UI as "Caller"
participant MF as "extract_video_metadata_mf"
participant CG as "ComGuard"
participant MG as "MFGuard"
participant SR as "IMFSourceReader"
UI->>MF : "Path"
MF->>CG : "CoInitializeEx (MTA)"
CG-->>MF : "OK or changed mode"
MF->>MG : "MFStartup(NOSOCKET)"
MG-->>MF : "OK"
MF->>SR : "MFCreateSourceReaderFromURL(PCWSTR)"
SR-->>MF : "IMFSourceReader"
MF->>SR : "GetPresentationAttribute(MF_PD_DURATION)"
SR-->>MF : "Duration (100ns)"
MF->>SR : "GetNativeMediaType(first video/audio)"
SR-->>MF : "IMFMediaType"
MF->>MF : "Read width/height, framerate, bitrate, codec GUID"
MF-->>UI : "VideoMetadataMF or None"
```

**Diagram sources**
- [media_foundation.rs:104-144](file://src/infrastructure/windows/media_foundation.rs#L104-L144)
- [media_foundation.rs:152-231](file://src/infrastructure/windows/media_foundation.rs#L152-L231)

**Section sources**
- [media_foundation.rs:1-439](file://src/infrastructure/windows/media_foundation.rs#L1-L439)

### COM Scope and Handle Management
RAII wrappers ensure proper COM initialization and uninitialization, preventing leaks.

```mermaid
classDiagram
class ComScope {
+bool initialized
+sta() ComScope
+is_initialized() bool
}
class ComGuard {
-bool initialized
+new() Option~Self~
+drop() void
}
class MFGuard {
-bool started
+new() Option~Self~
+drop() void
}
ComScope --> ComGuard : "similar pattern"
ComGuard <.. media_foundation.rs : "used in MF extraction"
MFGuard <.. media_foundation.rs : "used in MF extraction"
```

**Diagram sources**
- [com_scope.rs:1-41](file://src/infrastructure/windows/com_scope.rs#L1-L41)
- [media_foundation.rs:42-98](file://src/infrastructure/windows/media_foundation.rs#L42-L98)

**Section sources**
- [com_scope.rs:1-41](file://src/infrastructure/windows/com_scope.rs#L1-L41)
- [media_foundation.rs:42-98](file://src/infrastructure/windows/media_foundation.rs#L42-L98)

### Windows Service Integration (Search Service)
The search service integrates with Windows SCM, supports console mode, and runs an IPC server.

```mermaid
sequenceDiagram
participant SCM as "Windows SCM"
participant SVC as "mtt-search-service main"
participant CTRL as "SetConsoleCtrlHandler"
participant IPC as "ipc_server : : run_ipc_server"
participant IDX as "spawn_indexers_for_discovered_volumes"
SCM->>SVC : "Dispatch service"
SVC->>SVC : "match args (install|uninstall|run-console)"
alt "run-console"
SVC->>CTRL : "Install Ctrl+C handler"
CTRL-->>SVC : "OK"
SVC->>IDX : "Discover volumes and spawn indexers"
SVC->>IPC : "Start IPC server"
IPC-->>SVC : "Blocks until shutdown"
else "service"
SVC-->>SCM : "Run as service"
end
```

**Diagram sources**
- [main.rs:112-156](file://crates/mtt-search-service/src/main.rs#L112-L156)
- [main.rs:168-187](file://crates/mtt-search-service/src/main.rs#L168-L187)
- [main.rs:190-307](file://crates/mtt-search-service/src/main.rs#L190-L307)

**Section sources**
- [main.rs:112-307](file://crates/mtt-search-service/src/main.rs#L112-L307)
- [Cargo.toml:1-33](file://crates/mtt-search-service/Cargo.toml#L1-L33)

## Dependency Analysis
The Windows module re-exports are centralized in the module root, enabling clean imports across the application.

```mermaid
graph LR
MOD["windows/mod.rs"] --> CM["shell_operations/context_menu.rs"]
MOD --> FO["shell_operations/file_op.rs"]
MOD --> SF["shell_operations/shfile_ops.rs"]
MOD --> FS["file_system.rs"]
MOD --> DV["drives.rs"]
MOD --> SI["system_info.rs"]
MOD --> CR["codec_registry.rs"]
MOD --> MF["media_foundation.rs"]
MOD --> CS["com_scope.rs"]
```

**Diagram sources**
- [mod.rs:31-60](file://src/infrastructure/windows/mod.rs#L31-L60)

**Section sources**
- [mod.rs:1-60](file://src/infrastructure/windows/mod.rs#L1-L60)

## Performance Considerations
- Prefer IFileOperation for batch operations to reduce UI overhead and improve reliability
- Use LRU caching for codec name resolution to minimize registry and Media Foundation queries
- Avoid heavy I/O on Windows system paths to leverage Shell optimizations
- Use 64-bit free-space APIs for large volumes to prevent overflow
- Initialize COM once per thread and drop it deterministically to avoid resource leaks

## Troubleshooting Guide
- Context menu invocation failures: ensure COM is initialized with the correct threading model and that PIDLs are freed on all error paths
- File operation failures: fall back from IFileOperation to SHFileOperation and verify undo/confirmation flags
- Volume label rename failures: handle access denied by launching an elevated helper and checking exit codes
- Media Foundation initialization: handle RPC_E_CHANGED_MODE and ensure MFStartup/MFShutdown pairing
- Service startup: verify DLL search directory hardening and SCM dispatch paths

**Section sources**
- [context_menu.rs:37-59](file://src/infrastructure/windows/shell_operations/context_menu.rs#L37-L59)
- [file_op.rs:36-42](file://src/infrastructure/windows/shell_operations/file_op.rs#L36-L42)
- [drives.rs:193-276](file://src/infrastructure/windows/drives.rs#L193-L276)
- [media_foundation.rs:46-69](file://src/infrastructure/windows/media_foundation.rs#L46-L69)
- [main.rs:112-125](file://crates/mtt-search-service/src/main.rs#L112-L125)

## Conclusion
The project’s Windows API integration is structured around robust shell operations, safe COM usage, and resilient system utilities. The search service demonstrates production-grade Windows service patterns with IPC and indexing orchestration. Following the documented patterns ensures correctness, performance, and maintainability when extending Windows-specific functionality.