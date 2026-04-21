# Overall System Architecture

<cite>
**Referenced Files in This Document**
- [Cargo.toml](file://Cargo.toml)
- [src/lib.rs](file://src/lib.rs)
- [src/main.rs](file://src/main.rs)
- [src/app/mod.rs](file://src/app/mod.rs)
- [src/infrastructure/mod.rs](file://src/infrastructure/mod.rs)
- [src/workers/mod.rs](file://src/workers/mod.rs)
- [src/viewer_runtime.rs](file://src/viewer_runtime.rs)
- [crates/mtt-search-protocol/src/lib.rs](file://crates/mtt-search-protocol/src/lib.rs)
- [crates/mtt-search-service/src/main.rs](file://crates/mtt-search-service/src/main.rs)
- [crates/mtt-search-service/src/ipc_server/mod.rs](file://crates/mtt-search-service/src/ipc_server/mod.rs)
- [crates/mtt-search-service/src/ipc_authorization.rs](file://crates/mtt-search-service/src/ipc_authorization.rs)
- [docs/03_architecture.md](file://docs/03_architecture.md)
- [docs/04_module_map.md](file://docs/04_module_map.md)
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
This document describes the overall system architecture of MTT File Manager. It explains the layered architecture separating Presentation, Application, Domain, and Infrastructure concerns, documents the multi-process design (main GUI process, dedicated viewer processes, and background worker threads), and details the workspace structure with three crates: mtt-file-manager, mtt-search-protocol, and mtt-search-service. It also covers system boundaries, component relationships, and data flow patterns, and presents architectural diagrams to illustrate interactions between the UI, workers, infrastructure, and external services.

## Project Structure
The project is organized as a Cargo workspace with three crates:
- mtt-file-manager (binary): Main GUI application built with eframe/egui, hosting the primary UI and coordinating workers and infrastructure.
- mtt-search-protocol (library): Defines shared IPC types and serialization for communication with the search service.
- mtt-search-service (binary): A Windows service that performs hybrid indexing (USN-based and fallback scanning), persists indices, and exposes a named pipe server for search queries.

```mermaid
graph TB
subgraph "Workspace"
A["mtt-file-manager<br/>Binary + GUI"]
B["mtt-search-protocol<br/>Library"]
C["mtt-search-service<br/>Windows Service"]
end
A --> B
A -. uses IPC .-> C
C --> B
```

**Diagram sources**
- [Cargo.toml:1-137](file://Cargo.toml#L1-L137)
- [docs/03_architecture.md:1-20](file://docs/03_architecture.md#L1-L20)

**Section sources**
- [Cargo.toml:1-137](file://Cargo.toml#L1-L137)
- [docs/03_architecture.md:1-20](file://docs/03_architecture.md#L1-L20)
- [docs/04_module_map.md:291-322](file://docs/04_module_map.md#L291-L322)

## Core Components
- Presentation Layer: Built on eframe/egui, providing the immediate-mode UI with toolbars, tab bar, file list, sidebar, and preview panel. The main entry point initializes the window, GPU backend preferences, and optional viewer modes.
- Application Layer: Orchestrates navigation, file operations, clipboard, sorting, watchers, and UI state transitions. It coordinates background workers and integrates with infrastructure for file system operations and caching.
- Domain Layer: Encapsulates core domain entities such as FileEntry, DriveInfo, Thumbnail, SortMode, ViewMode, and error types (AppError).
- Infrastructure Layer: Provides Windows-specific integrations (Shell API, File System, Media Foundation, COM), persistence (SQLite), caching (disk and directory caches), and worker threading primitives. Also includes specialized modules for OneDrive, security, and virtual drive configuration.

Key runtime characteristics:
- Multi-process design: The main GUI process spawns dedicated viewer subprocesses for images, PDFs, text, and videos. Each viewer process uses a lightweight runtime to minimize resource usage.
- Background workers: Dedicated worker threads handle thumbnail extraction, file operations, prefetching, global search, and idle warmup. They communicate via channels and are designed for graceful shutdown.

**Section sources**
- [src/lib.rs:1-20](file://src/lib.rs#L1-L20)
- [src/main.rs:106-305](file://src/main.rs#L106-L305)
- [src/app/mod.rs:1-32](file://src/app/mod.rs#L1-L32)
- [src/infrastructure/mod.rs:1-26](file://src/infrastructure/mod.rs#L1-L26)
- [src/workers/mod.rs:1-9](file://src/workers/mod.rs#L1-L9)
- [src/viewer_runtime.rs:1-86](file://src/viewer_runtime.rs#L1-L86)

## Architecture Overview
MTT File Manager follows a layered architecture with clear separation of responsibilities:

```mermaid
graph TB
subgraph "Presentation Layer"
UI["UI Layer<br/>eframe/egui"]
end
subgraph "Application Layer"
APP["Application Logic<br/>Navigation, File Ops,<br/>Sorting, Watcher"]
end
subgraph "Domain Layer"
DOMAIN["Domain Entities<br/>FileEntry, Thumbnail,<br/>SortMode, ViewMode, Errors"]
end
subgraph "Infrastructure Layer"
INFRA["Windows Integration<br/>Shell API, FS Ops,<br/>Media Foundation, Persistence"]
WORKERS["Background Workers<br/>Thumbnail, File Ops,<br/>Prefetch, Global Search"]
end
subgraph "External Services"
SVC["Search Service<br/>Windows Service"]
VIEWERS["Standalone Viewers<br/>Image/PDF/Text/Video"]
end
UI --> APP
APP --> DOMAIN
APP --> INFRA
APP --> WORKERS
APP -. IPC .-> SVC
APP -. Spawns .-> VIEWERS
SVC --> INFRA
```

**Diagram sources**
- [docs/03_architecture.md:22-101](file://docs/03_architecture.md#L22-L101)
- [src/main.rs:144-215](file://src/main.rs#L144-L215)
- [crates/mtt-search-service/src/main.rs:112-307](file://crates/mtt-search-service/src/main.rs#L112-L307)

**Section sources**
- [docs/03_architecture.md:22-101](file://docs/03_architecture.md#L22-L101)

## Detailed Component Analysis

### Multi-Process Design
The system employs a multi-process architecture:
- Main GUI process: Initializes the eframe window, manages UI state, and coordinates workers. It supports optional viewer modes via command-line flags.
- Dedicated viewer processes: Separate subprocesses for image, PDF, text, and video viewing. Each uses a lightweight runtime to reduce startup overhead and memory footprint.
- Search service process: A Windows service that runs independently, performing indexing and exposing a named pipe interface for search queries.

```mermaid
sequenceDiagram
participant Main as "Main GUI Process"
participant ViewImg as "Image Viewer Subprocess"
participant ViewPdf as "PDF Viewer Subprocess"
participant ViewTxt as "Text Viewer Subprocess"
participant ViewVid as "Video Player Subprocess"
Main->>ViewImg : "--image-viewer <path>"
Main->>ViewPdf : "--pdf-viewer <path>"
Main->>ViewTxt : "--text-viewer <path>"
Main->>ViewVid : "--video-player <path> --position <sec> --volume <0..1>"
ViewImg-->>Main : "Displays image"
ViewPdf-->>Main : "Displays PDF"
ViewTxt-->>Main : "Displays text"
ViewVid-->>Main : "Plays video"
```

**Diagram sources**
- [src/main.rs:144-215](file://src/main.rs#L144-L215)
- [src/viewer_runtime.rs:75-85](file://src/viewer_runtime.rs#L75-L85)

**Section sources**
- [src/main.rs:144-215](file://src/main.rs#L144-L215)
- [src/viewer_runtime.rs:1-86](file://src/viewer_runtime.rs#L1-L86)

### Search Service and IPC
The search service runs as a separate Windows service, exposing a named pipe interface for the main application to query file metadata and status. The protocol crate defines strongly-typed requests and responses with validation limits to prevent abuse.

```mermaid
sequenceDiagram
participant App as "Main App"
participant Proto as "mtt-search-protocol"
participant Svc as "mtt-search-service"
participant IPC as "Named Pipe Server"
App->>Proto : "Encode SearchRequest"
App->>Svc : "Send via Named Pipe"
Svc->>IPC : "Accept client"
IPC-->>Svc : "Dispatch handler"
Svc-->>App : "SearchResponse (Results/Status/Error)"
```

**Diagram sources**
- [crates/mtt-search-protocol/src/lib.rs:1-290](file://crates/mtt-search-protocol/src/lib.rs#L1-L290)
- [crates/mtt-search-service/src/main.rs:112-307](file://crates/mtt-search-service/src/main.rs#L112-L307)
- [crates/mtt-search-service/src/ipc_server/mod.rs:68-104](file://crates/mtt-search-service/src/ipc_server/mod.rs#L68-L104)

**Section sources**
- [crates/mtt-search-protocol/src/lib.rs:1-290](file://crates/mtt-search-protocol/src/lib.rs#L1-L290)
- [crates/mtt-search-service/src/main.rs:112-307](file://crates/mtt-search-service/src/main.rs#L112-L307)
- [crates/mtt-search-service/src/ipc_server/mod.rs:68-104](file://crates/mtt-search-service/src/ipc_server/mod.rs#L68-L104)

### IPC Authorization and Security
The search service enforces client authorization to ensure only authorized clients can access the pipe. It impersonates the named pipe client to evaluate permissions against requested paths and reverts to self afterward.

```mermaid
flowchart TD
Start(["Client connects"]) --> Impersonate["ImpersonateNamedPipeClient"]
Impersonate --> Check["Check client can read requested path"]
Check --> |Allowed| Proceed["Process request"]
Check --> |Denied| Deny["Reject request"]
Proceed --> Revert["RevertToSelf"]
Deny --> Revert
Revert --> End(["Close connection"])
```

**Diagram sources**
- [crates/mtt-search-service/src/ipc_authorization.rs:30-76](file://crates/mtt-search-service/src/ipc_authorization.rs#L30-L76)

**Section sources**
- [crates/mtt-search-service/src/ipc_authorization.rs:30-76](file://crates/mtt-search-service/src/ipc_authorization.rs#L30-L76)

### Background Workers and Threading
The application spawns dedicated worker threads for tasks such as thumbnail extraction, file operations, prefetching, and global search. These workers are designed for graceful shutdown by dropping their senders, which causes receive loops to exit deterministically.

```mermaid
flowchart TD
Init(["Init Workers"]) --> Spawn["Spawn Worker Threads"]
Spawn --> Loop{"Receive Request"}
Loop --> |Ok| Handle["Handle Task"]
Loop --> |Err| Exit["Exit Thread"]
Handle --> Loop
Exit --> Cleanup["Drop Senders on Shutdown"]
```

**Diagram sources**
- [src/app/operations/shutdown.rs:1-35](file://src/app/operations/shutdown.rs#L1-L35)
- [src/workers/mod.rs:1-9](file://src/workers/mod.rs#L1-L9)

**Section sources**
- [src/app/operations/shutdown.rs:1-35](file://src/app/operations/shutdown.rs#L1-L35)
- [src/workers/mod.rs:1-9](file://src/workers/mod.rs#L1-L9)

## Dependency Analysis
The workspace dependencies define the relationship between crates and shared libraries. The main application depends on the search protocol crate and integrates Windows APIs and third-party libraries for UI, media, and file operations.

```mermaid
graph LR
FM["mtt-file-manager"] --> SP["mtt-search-protocol"]
FM --> WS["Windows SDK (features)"]
FM --> EXT["External Libraries<br/>eframe, rayon, image, rusqlite, ..."]
SS["mtt-search-service"] --> SP
SS --> WS
```

**Diagram sources**
- [Cargo.toml:1-137](file://Cargo.toml#L1-L137)
- [crates/mtt-search-service/Cargo.toml:1-33](file://crates/mtt-search-service/Cargo.toml#L1-L33)
- [crates/mtt-search-protocol/Cargo.toml:1-9](file://crates/mtt-search-protocol/Cargo.toml#L1-L9)

**Section sources**
- [Cargo.toml:1-137](file://Cargo.toml#L1-L137)
- [crates/mtt-search-service/Cargo.toml:1-33](file://crates/mtt-search-service/Cargo.toml#L1-L33)
- [crates/mtt-search-protocol/Cargo.toml:1-9](file://crates/mtt-search-protocol/Cargo.toml#L1-L9)

## Performance Considerations
- GPU backend selection: The main GUI process selects a high-performance GPU backend and minimizes frame latency by limiting queued frames.
- Lightweight viewers: Standalone viewers use a minimal runtime and a simpler renderer to reduce memory usage and improve responsiveness.
- Background worker design: Workers use channels and atomic flags for graceful shutdown, preventing resource leaks and ensuring clean termination.
- IPC limits: The protocol enforces maximum payload sizes and result counts to mitigate abuse and protect system resources.
- DLL search hardening: Both the main process and the search service remove the current working directory from the default DLL search order to prevent DLL planting attacks.

[No sources needed since this section provides general guidance]

## Troubleshooting Guide
- Logging levels: The main process adjusts log verbosity depending on whether a console is attached to avoid excessive allocations on background threads.
- Graceful shutdown: The application disconnects worker senders during shutdown to signal threads to exit cleanly.
- IPC connectivity: The image viewer IPC server retries connecting to the pipe with backoff and validates payloads to prevent injection attacks.
- Service mode: The search service supports installation, uninstallation, and console modes for debugging and deployment.

**Section sources**
- [src/main.rs:121-141](file://src/main.rs#L121-L141)
- [src/app/operations/shutdown.rs:1-35](file://src/app/operations/shutdown.rs#L1-L35)
- [src/image_viewer/ipc.rs:67-296](file://src/image_viewer/ipc.rs#L67-L296)
- [crates/mtt-search-service/src/main.rs:112-156](file://crates/mtt-search-service/src/main.rs#L112-L156)

## Conclusion
MTT File Manager’s architecture cleanly separates concerns across Presentation, Application, Domain, and Infrastructure layers, while leveraging a multi-process design for scalability and maintainability. The main GUI process coordinates extensive background workers, integrates with Windows subsystems, and communicates with a dedicated search service via a secure named pipe interface. The workspace is structured around three crates that encapsulate the GUI, IPC protocol, and the search service, enabling clear boundaries and focused responsibilities. These design choices support performance, scalability, and robustness across diverse file management scenarios.