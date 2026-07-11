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
            NotificationLevel::Info => Color32::from_rgb(30, 58, 95), // Dark blue
            NotificationLevel::Success => Color32::from_rgb(20, 70, 45), // Dark green
            NotificationLevel::Warning => Color32::from_rgb(80, 60, 10), // Dark amber
            NotificationLevel::Error => Color32::from_rgb(95, 25, 25), // Dark red
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
    pub id: u64,
    pub message: String,
    pub level: NotificationLevel,
    pub created_at: Instant,
    pub duration: Option<Duration>,
    pub dismissible: bool,
    key: Option<&'static str>,
}

impl AppNotification {
    /// Creates a new notification
    pub fn new(message: impl Into<String>, level: NotificationLevel) -> Self {
        Self {
            id: 0,
            message: message.into(),
            level,
            created_at: Instant::now(),
            duration: Some(Duration::from_secs(6)),
            dismissible: false,
            key: None,
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
        self.duration = Some(duration);
        self
    }

    fn persistent(mut self, key: &'static str) -> Self {
        self.duration = None;
        self.dismissible = true;
        self.key = Some(key);
        self
    }

    /// Check if the notification has expired
    pub fn is_expired(&self) -> bool {
        self.duration
            .is_some_and(|duration| self.created_at.elapsed() >= duration)
    }

    /// Get remaining time as a fraction (0.0 - 1.0)
    pub fn remaining_fraction(&self) -> f32 {
        self.duration.map_or(1.0, |duration| {
            let elapsed = self.created_at.elapsed().as_secs_f32();
            let total = duration.as_secs_f32();
            (1.0 - (elapsed / total)).clamp(0.0, 1.0)
        })
    }

    pub fn needs_expiration_repaint(&self) -> bool {
        self.duration.is_some()
    }
}

/// Notification manager for the application
pub struct NotificationManager {
    notifications: Vec<AppNotification>,
    next_id: u64,
}

impl Default for NotificationManager {
    fn default() -> Self {
        Self::new()
    }
}

impl NotificationManager {
    pub fn new() -> Self {
        Self {
            notifications: Vec::new(),
            next_id: 1,
        }
    }

    /// Add a new notification
    pub fn push(&mut self, mut notification: AppNotification) {
        notification.id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
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

    /// Replaces the existing persistent notification for a feature instead of stacking alerts.
    pub fn persistent_warning(&mut self, key: &'static str, message: impl Into<String>) {
        let message = message.into();
        if let Some(notification) = self
            .notifications
            .iter_mut()
            .find(|notification| notification.key == Some(key))
        {
            notification.message = message;
            notification.level = NotificationLevel::Warning;
            notification.created_at = Instant::now();
            return;
        }
        self.push(AppNotification::warning(message).persistent(key));
    }

    /// Remove expired notifications
    pub fn cleanup(&mut self) {
        self.notifications.retain(|n| !n.is_expired());
    }

    pub fn dismiss(&mut self, id: u64) {
        self.notifications
            .retain(|notification| notification.id != id);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persistent_notification_is_replaced_and_only_closes_when_dismissed() {
        let mut notifications = NotificationManager::new();
        notifications.persistent_warning("organizer_issues", "first issue");
        let id = notifications.active()[0].id;

        notifications.persistent_warning("organizer_issues", "updated issue");

        assert_eq!(notifications.active().len(), 1);
        assert_eq!(notifications.active()[0].message, "updated issue");
        assert!(!notifications.active()[0].is_expired());
        assert!(notifications.active()[0].dismissible);

        notifications.dismiss(id);
        assert!(notifications.is_empty());
    }
}
