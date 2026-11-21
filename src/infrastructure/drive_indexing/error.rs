#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriveIndexingErrorKind {
    Unsupported,
    AccessDenied,
    Transport,
    Busy,
    Service,
    Protocol,
    InvalidDrive,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriveIndexingError {
    pub kind: DriveIndexingErrorKind,
    pub message: String,
}

impl DriveIndexingError {
    pub fn new(kind: DriveIndexingErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn transport(message: impl Into<String>) -> Self {
        Self::new(DriveIndexingErrorKind::Transport, message)
    }

    pub fn protocol(message: impl Into<String>) -> Self {
        Self::new(DriveIndexingErrorKind::Protocol, message)
    }

    pub fn service(message: String) -> Self {
        let kind = match message.as_str() {
            "Authorization failed" => DriveIndexingErrorKind::AccessDenied,
            "Server busy, try again later" => DriveIndexingErrorKind::Busy,
            _ => DriveIndexingErrorKind::Service,
        };
        Self::new(kind, message)
    }

    pub fn may_have_applied(&self) -> bool {
        matches!(
            self.kind,
            DriveIndexingErrorKind::Transport | DriveIndexingErrorKind::Protocol
        )
    }
}

impl std::fmt::Display for DriveIndexingError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for DriveIndexingError {}
