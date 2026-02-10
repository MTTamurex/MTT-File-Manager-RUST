//! Centralized application error types.

use std::path::PathBuf;
use thiserror::Error;

/// Main application error enum.
#[derive(Error, Debug)]
pub enum AppError {
    #[error("Security error: {0}")]
    Security(#[from] crate::infrastructure::security::SecurityError),

    #[error("Windows API error: {0}")]
    WindowsApi(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Thumbnail extraction failed for {path}: {source}")]
    ThumbnailExtraction {
        path: PathBuf,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    #[error("File operation failed: {0}")]
    FileOperation(String),

    #[error("Invalid state: {0}")]
    InvalidState(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Worker thread error: {0}")]
    Worker(String),

    #[error("UI rendering error: {0}")]
    UiRendering(String),
}

/// Common application result type.
pub type AppResult<T> = Result<T, AppError>;

/// Helper to create Windows API error.
pub fn windows_error(message: &str) -> AppError {
    AppError::WindowsApi(message.to_string())
}

/// Helper to create file operation error.
pub fn file_operation_error(message: &str) -> AppError {
    AppError::FileOperation(message.to_string())
}

/// Helper to create invalid-state error.
pub fn invalid_state_error(message: &str) -> AppError {
    AppError::InvalidState(message.to_string())
}

/// Helper to create configuration error.
pub fn config_error(message: &str) -> AppError {
    AppError::Config(message.to_string())
}

/// Helper to create worker error.
pub fn worker_error(message: &str) -> AppError {
    AppError::Worker(message.to_string())
}

/// Helper to create UI rendering error.
pub fn ui_rendering_error(message: &str) -> AppError {
    AppError::UiRendering(message.to_string())
}

/// Macro to replace `unwrap()` with structured error propagation.
#[macro_export]
macro_rules! safe_unwrap {
    ($expr:expr, $context:expr) => {
        match $expr {
            Ok(val) => val,
            Err(e) => {
                eprintln!("[ERROR] {}: {:?}", $context, e);
                return Err($crate::domain::errors::AppError::from(e));
            }
        }
    };

    ($expr:expr, $context:expr, $default:expr) => {
        match $expr {
            Ok(val) => val,
            Err(e) => {
                eprintln!("[WARN] {}: {:?}, using default", $context, e);
                $default
            }
        }
    };
}

/// Macro to replace `expect()` with structured error propagation.
#[macro_export]
macro_rules! safe_expect {
    ($expr:expr, $message:expr) => {
        match $expr {
            Some(val) => val,
            None => {
                eprintln!("[ERROR] {}", $message);
                return Err($crate::domain::errors::AppError::InvalidState(
                    $message.to_string(),
                ));
            }
        }
    };
}

/// Extension trait to convert `Option<T>` into `AppResult<T>` with context.
pub trait OptionExt<T> {
    fn ok_or_app_error(self, context: &str) -> AppResult<T>;
}

impl<T> OptionExt<T> for Option<T> {
    fn ok_or_app_error(self, context: &str) -> AppResult<T> {
        self.ok_or_else(|| {
            let error = AppError::InvalidState(context.to_string());
            eprintln!("[ERROR] {:?}", error);
            error
        })
    }
}

/// Extension trait to map generic `Result<T, E>` into `AppResult<T>`.
pub trait ResultExt<T, E> {
    fn map_to_app_error(self, context: &str) -> AppResult<T>;
}

impl<T, E: std::error::Error + Send + Sync + 'static> ResultExt<T, E> for Result<T, E> {
    fn map_to_app_error(self, context: &str) -> AppResult<T> {
        self.map_err(|e| {
            let error = AppError::InvalidState(format!("{}: {}", context, e));
            eprintln!("[ERROR] {:?}", error);
            error
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_option_ext() {
        let some_value: Option<i32> = Some(42);
        let none_value: Option<i32> = None;

        assert!(some_value.ok_or_app_error("test").is_ok());
        assert!(none_value.ok_or_app_error("test").is_err());
    }

    #[test]
    fn test_windows_error_helper() {
        let error = windows_error("Failed to get icon");
        match error {
            AppError::WindowsApi(msg) => assert_eq!(msg, "Failed to get icon"),
            _ => panic!("Wrong error type"),
        }
    }

    fn unwrap_err_case() -> AppResult<()> {
        let bad_result: Result<i32, std::io::Error> =
            Err(std::io::Error::other("test error"));
        let _ = safe_unwrap!(bad_result, "testing error");
        Ok(())
    }

    #[test]
    fn test_safe_unwrap_macro() {
        let result: Result<i32, std::io::Error> = Ok(42);
        let value = safe_unwrap!(result, "testing", -1);
        assert_eq!(value, 42);
        assert!(unwrap_err_case().is_err());
    }
}
