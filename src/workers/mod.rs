// Background workers for async operations

pub mod folder_preview_worker;
pub mod folder_scanner;
pub mod thumbnail_loader;
pub mod thumbnail_worker;
pub mod file_operation_worker;
pub mod idle_warmup;
pub mod prefetch_worker;
pub mod predictive_prefetch;
// pub mod batch_thumbnail_loader; // Not used currently
