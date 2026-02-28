//! Notification system for displaying toast messages
//! Follows .cursorrules: single responsibility, < 150 lines

use std::time::{Duration, Instant};

/// Notification severity level
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NotificationLevel {
    Info,
    Success,
    Warning,
    Error,
}

impl NotificationLevel {
    /// Get the background color for this notification level (dark, high-contrast)
    pub fn color(&self) -> eframe::egui::Color32 {
        use eframe::egui::Color32;
        match self {
            NotificationLevel::Info => Color32::from_rgb(30, 58, 95),    // Dark blue
            NotificationLevel::Success => Color32::from_rgb(20, 70, 45), // Dark green
            NotificationLevel::Warning => Color32::from_rgb(80, 60, 10), // Dark amber
            NotificationLevel::Error => Color32::from_rgb(95, 25, 25),   // Dark red
        }
    }

    /// Get the accent/border color for this notification level
    pub fn accent_color(&self) -> eframe::egui::Color32 {
        use eframe::egui::Color32;
        match self {
            NotificationLevel::Info => Color32::from_rgb(100, 160, 255),
            NotificationLevel::Success => Color32::from_rgb(80, 200, 120),
            NotificationLevel::Warning => Color32::from_rgb(240, 190, 50),
            NotificationLevel::Error => Color32::from_rgb(240, 90, 90),
        }
    }

    /// Get icon for this notification level
    pub fn icon(&self) -> &'static str {
        match self {
            NotificationLevel::Info => "ℹ",
            NotificationLevel::Success => "✓",
            NotificationLevel::Warning => "⚠",
            NotificationLevel::Error => "✕",
        }
    }
}

/// A single notification message
#[derive(Clone, Debug)]
pub struct AppNotification {
    pub message: String,
    pub level: NotificationLevel,
    pub created_at: Instant,
    pub duration: Duration,
}

impl AppNotification {
    /// Creates a new notification
    pub fn new(message: impl Into<String>, level: NotificationLevel) -> Self {
        Self {
            message: message.into(),
            level,
            created_at: Instant::now(),
            duration: Duration::from_secs(6),
        }
    }

    /// Creates an info notification
    pub fn info(message: impl Into<String>) -> Self {
        Self::new(message, NotificationLevel::Info)
    }

    /// Creates a success notification
    pub fn success(message: impl Into<String>) -> Self {
        Self::new(message, NotificationLevel::Success)
    }

    /// Creates a warning notification
    pub fn warning(message: impl Into<String>) -> Self {
        Self::new(message, NotificationLevel::Warning)
    }

    /// Creates an error notification
    pub fn error(message: impl Into<String>) -> Self {
        Self::new(message, NotificationLevel::Error)
    }

    /// With custom duration
    pub fn with_duration(mut self, duration: Duration) -> Self {
        self.duration = duration;
        self
    }

    /// Check if the notification has expired
    pub fn is_expired(&self) -> bool {
        self.created_at.elapsed() >= self.duration
    }

    /// Get remaining time as a fraction (0.0 - 1.0)
    pub fn remaining_fraction(&self) -> f32 {
        let elapsed = self.created_at.elapsed().as_secs_f32();
        let total = self.duration.as_secs_f32();
        (1.0 - (elapsed / total)).clamp(0.0, 1.0)
    }
}

/// Notification manager for the application
#[derive(Default)]
pub struct NotificationManager {
    notifications: Vec<AppNotification>,
}

impl NotificationManager {
    pub fn new() -> Self {
        Self {
            notifications: Vec::new(),
        }
    }

    /// Add a new notification
    pub fn push(&mut self, notification: AppNotification) {
        self.notifications.push(notification);
    }

    /// Add info notification
    pub fn info(&mut self, message: impl Into<String>) {
        self.push(AppNotification::info(message));
    }

    /// Add success notification
    pub fn success(&mut self, message: impl Into<String>) {
        self.push(AppNotification::success(message));
    }

    /// Add warning notification
    pub fn warning(&mut self, message: impl Into<String>) {
        self.push(AppNotification::warning(message));
    }

    /// Add error notification
    pub fn error(&mut self, message: impl Into<String>) {
        self.push(AppNotification::error(message));
    }

    /// Remove expired notifications
    pub fn cleanup(&mut self) {
        self.notifications.retain(|n| !n.is_expired());
    }

    /// Get active notifications (for rendering)
    pub fn active(&self) -> &[AppNotification] {
        &self.notifications
    }

    /// Check if there are any notifications
    pub fn is_empty(&self) -> bool {
        self.notifications.is_empty()
    }
}
