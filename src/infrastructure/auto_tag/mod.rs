//! Auto-tag: watches rule folders and assigns their tag to anything that
//! appears inside them (recursively), mirroring the organizer's watcher
//! pattern (notify + stability delay) without the move machinery.

use crate::domain::auto_tag_rule::AutoTagRule;
use crate::domain::file_tag::FileTag;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Sender;
use std::sync::Arc;

mod manager;
#[cfg(feature = "notify-watcher")]
mod worker;

pub use manager::AutoTagManager;

/// Commands the UI sends to the worker thread.
pub(crate) enum AutoTagCommand {
    /// Full refresh of rules and tag definitions (any rule/tag mutation).
    RulesUpdated {
        rules: Vec<AutoTagRule>,
        defs: Vec<FileTag>,
    },
    /// Re-scan a rule's folder and tag everything currently inside it.
    ApplyExisting {
        rule_id: i64,
    },
    Shutdown,
}

/// Status reported per rule so the settings UI can show watcher health.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutoTagRuleStatus {
    Watching,
    FolderMissing,
}

/// Events the worker sends back to the UI thread.
pub enum AutoTagEvent {
    /// Tags were written to the database for these paths (the UI mirrors them
    /// into its in-memory assignment state and heals any ancestry staleness).
    Applied {
        assignments: Vec<(PathBuf, i64)>,
    },
    Status {
        rule_id: i64,
        status: AutoTagRuleStatus,
    },
    WatcherError {
        message: String,
    },
}

/// Shared worker context handed to [`worker::run_auto_tag`].
pub(crate) struct AutoTagContext {
    pub app_state_db: Arc<crate::infrastructure::app_state_db::AppStateDb>,
    pub event_sender: Sender<AutoTagEvent>,
    pub shutdown: Arc<AtomicBool>,
}
