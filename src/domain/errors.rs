//! Erros centralizados do MTT File Manager
//! Segue as regras do .cursorrules: totalidade (funções não mentem sobre erros)

use std::path::PathBuf;
use thiserror::Error;

/// Erros principais da aplicação
#[derive(Error, Debug)]
pub enum AppError {
    #[error("Security error: {0}")]
    Security(#[from] crate::infrastructure::security::SecurityError),
    
    #[error("Windows API error: {0}")]
    WindowsApi(String),
    
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    
    #[error("Thumbnail extraction failed for {path}: {source}")]
    ThumbnailExtraction {
        path: PathBuf,
        #[source]
        source: anyhow::Error,
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

/// Tipo de resultado padrão da aplicação
pub type AppResult<T> = Result<T, AppError>;

/// Helper para converter erros do Windows em AppError
pub fn windows_error(message: &str) -> AppError {
    AppError::WindowsApi(message.to_string())
}

/// Helper para erros de operação de arquivo
pub fn file_operation_error(message: &str) -> AppError {
    AppError::FileOperation(message.to_string())
}

/// Helper para erros de estado inválido
pub fn invalid_state_error(message: &str) -> AppError {
    AppError::InvalidState(message.to_string())
}

/// Helper para erros de configuração
pub fn config_error(message: &str) -> AppError {
    AppError::Config(message.to_string())
}

/// Helper para erros de worker threads
pub fn worker_error(message: &str) -> AppError {
    AppError::Worker(message.to_string())
}

/// Helper para erros de renderização UI
pub fn ui_rendering_error(message: &str) -> AppError {
    AppError::UiRendering(message.to_string())
}

/// Macro para substituir .unwrap() com logging
#[macro_export]
macro_rules! safe_unwrap {
    ($expr:expr, $context:expr) => {
        match $expr {
            Ok(val) => val,
            Err(e) => {
                tracing::error!("{}: {:?}", $context, e);
                return Err(AppError::from(e));
            }
        }
    };
    
    ($expr:expr, $context:expr, $default:expr) => {
        match $expr {
            Ok(val) => val,
            Err(e) => {
                tracing::warn!("{}: {:?}, using default", $context, e);
                $default
            }
        }
    };
}

/// Macro para substituir .expect() com contexto rico
#[macro_export]
macro_rules! safe_expect {
    ($expr:expr, $message:expr) => {
        match $expr {
            Some(val) => val,
            None => {
                tracing::error!("{}", $message);
                return Err(AppError::InvalidState($message.to_string()));
            }
        }
    };
}

/// Trait para converter Option em AppResult com contexto
pub trait OptionExt<T> {
    fn ok_or_app_error(self, context: &str) -> AppResult<T>;
}

impl<T> OptionExt<T> for Option<T> {
    fn ok_or_app_error(self, context: &str) -> AppResult<T> {
        self.ok_or_else(|| {
            let error = AppError::InvalidState(context.to_string());
            tracing::error!("{:?}", error);
            error
        })
    }
}

/// Trait para converter Result com qualquer erro em AppResult
pub trait ResultExt<T, E> {
    fn map_to_app_error(self, context: &str) -> AppResult<T>;
}

impl<T, E: std::error::Error + Send + Sync + 'static> ResultExt<T, E> for Result<T, E> {
    fn map_to_app_error(self, context: &str) -> AppResult<T> {
        self.map_err(|e| {
            let error = AppError::InvalidState(format!("{}: {}", context, e));
            tracing::error!("{:?}", error);
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
    
    #[test]
    fn test_safe_unwrap_macro() {
        let result: Result<i32, std::io::Error> = Ok(42);
        let value = safe_unwrap!(result, "testing");
        assert_eq!(value, 42);
        
        let bad_result: Result<i32, std::io::Error> = 
            Err(std::io::Error::new(std::io::ErrorKind::Other, "test error"));
        let _ = safe_unwrap!(bad_result, "testing error"); // Deve retornar Err
    }
}
