use serde::{Deserialize, Serialize};

pub const MAX_DRIVE_HEALTH_STRING_LEN: usize = 256;
pub const MAX_DRIVE_HEALTH_FEATURES: usize = 64;
pub const MAX_DRIVE_HEALTH_FEATURE_LEN: usize = 128;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum DriveHealthState {
    Unknown,
    Good,
    Warning,
    Critical,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum DriveRotation {
    Unknown,
    SolidState,
    Rpm(u16),
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct DriveHealthSnapshot {
    pub drive_letter: char,
    pub physical_disk_number: u32,
    pub model: Option<String>,
    pub serial_number: Option<String>,
    pub firmware_revision: Option<String>,
    pub interface: Option<String>,
    pub temperature_celsius: Option<i16>,
    pub life_remaining_percent: Option<u8>,
    pub total_host_reads_bytes: Option<u128>,
    pub total_host_writes_bytes: Option<u128>,
    pub power_cycle_count: Option<u128>,
    pub power_on_hours: Option<u128>,
    pub rotation: DriveRotation,
    pub current_transfer_mode: Option<String>,
    pub max_transfer_mode: Option<String>,
    pub standard: Option<String>,
    pub features: Vec<String>,
    pub health_state: DriveHealthState,
    pub smart_available: bool,
}

impl DriveHealthSnapshot {
    pub fn validate(&self) -> Result<(), String> {
        if !self.drive_letter.is_ascii_alphabetic() {
            return Err("drive health letter must be an ASCII letter".to_string());
        }

        for (name, value) in [
            ("model", &self.model),
            ("serial number", &self.serial_number),
            ("firmware revision", &self.firmware_revision),
            ("interface", &self.interface),
            ("current transfer mode", &self.current_transfer_mode),
            ("maximum transfer mode", &self.max_transfer_mode),
            ("standard", &self.standard),
        ] {
            if let Some(value) = value {
                validate_text(name, value, MAX_DRIVE_HEALTH_STRING_LEN)?;
            }
        }

        if self.features.len() > MAX_DRIVE_HEALTH_FEATURES {
            return Err(format!(
                "too many drive features ({}, max {})",
                self.features.len(),
                MAX_DRIVE_HEALTH_FEATURES
            ));
        }
        for feature in &self.features {
            if feature.is_empty() {
                return Err("drive feature is empty".to_string());
            }
            validate_text("drive feature", feature, MAX_DRIVE_HEALTH_FEATURE_LEN)?;
        }
        if self.life_remaining_percent.is_some_and(|life| life > 100) {
            return Err("drive life remaining percentage exceeds 100".to_string());
        }
        if matches!(self.rotation, DriveRotation::Rpm(0)) {
            return Err("drive rotation speed must be nonzero".to_string());
        }
        if !self.smart_available && self.health_state != DriveHealthState::Unknown {
            return Err("drive health state requires SMART data".to_string());
        }
        Ok(())
    }
}

fn validate_text(name: &str, value: &str, max_len: usize) -> Result<(), String> {
    if value.len() > max_len {
        return Err(format!(
            "{} too long ({} bytes, max {})",
            name,
            value.len(),
            max_len
        ));
    }
    if value.chars().any(char::is_control) {
        return Err(format!("{} contains control characters", name));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{decode_message, encode_message, SearchRequest, SearchResponse};

    fn sample_snapshot() -> DriveHealthSnapshot {
        DriveHealthSnapshot {
            drive_letter: 'C',
            physical_disk_number: 0,
            model: Some("Example NVMe".to_string()),
            serial_number: Some("SN123".to_string()),
            firmware_revision: Some("1.0".to_string()),
            interface: Some("NVMe".to_string()),
            temperature_celsius: Some(34),
            life_remaining_percent: Some(97),
            total_host_reads_bytes: Some(123_456_789_012_345_678_901),
            total_host_writes_bytes: Some(987_654_321),
            power_cycle_count: Some(42),
            power_on_hours: Some(1_234),
            rotation: DriveRotation::SolidState,
            current_transfer_mode: None,
            max_transfer_mode: None,
            standard: Some("NVMe 2.0".to_string()),
            features: vec!["SMART".to_string(), "TRIM".to_string()],
            health_state: DriveHealthState::Good,
            smart_available: true,
        }
    }

    #[test]
    fn request_and_response_roundtrip() {
        let request = SearchRequest::GetDriveHealth { drive_letter: 'd' };
        let encoded = encode_message(&request).unwrap();
        let decoded: SearchRequest = decode_message(&encoded[4..]).unwrap();
        assert!(matches!(
            decoded,
            SearchRequest::GetDriveHealth { drive_letter: 'd' }
        ));

        let snapshot = sample_snapshot();
        let encoded = encode_message(&SearchResponse::DriveHealth(snapshot.clone())).unwrap();
        let decoded: SearchResponse = decode_message(&encoded[4..]).unwrap();
        assert_eq!(decoded.validate(), Ok(()));
        assert!(matches!(
            decoded,
            SearchResponse::DriveHealth(decoded_snapshot) if decoded_snapshot == snapshot
        ));
    }

    #[test]
    fn request_rejects_non_ascii_letter() {
        assert!(SearchRequest::GetDriveHealth { drive_letter: '1' }
            .validate()
            .is_err());
        assert!(SearchRequest::GetDriveHealth { drive_letter: 'Ç' }
            .validate()
            .is_err());
    }

    #[test]
    fn response_rejects_oversized_or_control_text() {
        let mut snapshot = sample_snapshot();
        snapshot.model = Some("x".repeat(MAX_DRIVE_HEALTH_STRING_LEN + 1));
        assert!(snapshot.validate().is_err());

        let mut snapshot = sample_snapshot();
        snapshot.features = vec!["SMART\nforged".to_string()];
        assert!(snapshot.validate().is_err());

        let mut snapshot = sample_snapshot();
        snapshot.features = vec!["SMART".to_string(); MAX_DRIVE_HEALTH_FEATURES + 1];
        assert!(snapshot.validate().is_err());
    }

    #[test]
    fn response_rejects_invalid_health_invariants() {
        let mut snapshot = sample_snapshot();
        snapshot.life_remaining_percent = Some(101);
        assert!(snapshot.validate().is_err());

        let mut snapshot = sample_snapshot();
        snapshot.rotation = DriveRotation::Rpm(0);
        assert!(snapshot.validate().is_err());

        let mut snapshot = sample_snapshot();
        snapshot.smart_available = false;
        assert!(snapshot.validate().is_err());
    }
}
