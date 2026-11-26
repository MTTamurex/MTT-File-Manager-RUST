//! How the docked (detail panel) MPV video surface is hosted, plus diagnostic
//! overrides read from environment variables.
//!
//! The video renders through mpv's own D3D11 swapchain into a native window.
//! As a `WS_CHILD` of the main window, DWM re-composed the whole main window at
//! the video's cadence and scrolling dropped from 60 to ~46-51 fps on both the
//! integrated and the discrete GPU. Hosting it in an owned popup (composed on
//! its own, like the detached player) kept scrolling at a steady 60 fps.

pub(crate) const HOST_ENV: &str = "MTT_MPV_PREVIEW_HOST";
pub(crate) const FLIP_ENV: &str = "MTT_MPV_PREVIEW_FLIP";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PreviewHostTuning {
    /// Owned top-level popup (default) instead of a child window
    /// (`MTT_MPV_PREVIEW_HOST=child`, the previous behavior).
    pub popup: bool,
    /// `Some(false)` sets mpv `d3d11-flip=no` (blt-model swapchain).
    pub flip: Option<bool>,
}

impl Default for PreviewHostTuning {
    fn default() -> Self {
        Self {
            popup: true,
            flip: None,
        }
    }
}

impl PreviewHostTuning {
    pub(crate) fn from_env() -> Self {
        static TUNING: std::sync::OnceLock<PreviewHostTuning> = std::sync::OnceLock::new();
        *TUNING.get_or_init(|| Self::parse(|name| std::env::var(name).ok()))
    }

    pub(crate) fn parse(get: impl Fn(&str) -> Option<String>) -> Self {
        let value = |name: &str| get(name).map(|v| v.trim().to_ascii_lowercase());
        Self {
            popup: value(HOST_ENV).as_deref() != Some("child"),
            flip: match value(FLIP_ENV).as_deref() {
                Some("0" | "no") => Some(false),
                Some("1" | "yes") => Some(true),
                _ => None,
            },
        }
    }

    pub(crate) fn summary(self) -> String {
        format!(
            "host={} flip={}",
            if self.popup { "popup" } else { "child" },
            match self.flip {
                Some(true) => "yes",
                Some(false) => "no",
                None => "default",
            }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_popup_host_and_mpv_defaults() {
        let tuning = PreviewHostTuning::parse(|_| None);
        assert_eq!(tuning, PreviewHostTuning::default());
        assert!(tuning.popup);
    }

    #[test]
    fn child_host_remains_available() {
        let tuning = PreviewHostTuning::parse(|name| (name == HOST_ENV).then(|| "child".into()));
        assert!(!tuning.popup);
    }

    #[test]
    fn parses_every_knob() {
        let tuning = PreviewHostTuning::parse(|name| {
            Some(
                match name {
                    HOST_ENV => "Popup",
                    FLIP_ENV => "no",
                    _ => return None,
                }
                .to_string(),
            )
        });
        assert!(tuning.popup);
        assert_eq!(tuning.flip, Some(false));
    }
}
