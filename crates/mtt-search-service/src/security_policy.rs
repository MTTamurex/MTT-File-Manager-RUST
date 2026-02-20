#[derive(Debug, Clone, Copy)]
pub struct IpcSecurityPolicy {
    pub redact_status_metrics: bool,
}

impl IpcSecurityPolicy {
    pub fn from_env() -> Self {
        Self {
            redact_status_metrics: env_flag_enabled("MTT_SEARCH_REDACT_STATUS_METRICS"),
        }
    }
}

fn env_flag_enabled(name: &str) -> bool {
    match std::env::var(name) {
        Ok(value) => matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn env_flag_enabled_accepts_truthy_values() {
        assert!(matches_truthy("1"));
        assert!(matches_truthy("true"));
        assert!(matches_truthy("YES"));
        assert!(matches_truthy("on"));
    }

    #[test]
    fn env_flag_enabled_rejects_other_values() {
        assert!(!matches_truthy("0"));
        assert!(!matches_truthy("false"));
        assert!(!matches_truthy("off"));
        assert!(!matches_truthy("abc"));
    }

    fn matches_truthy(value: &str) -> bool {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    }
}