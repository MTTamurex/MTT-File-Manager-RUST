use mtt_search_protocol::{DriveHealthState, DriveRotation};

pub(super) struct NvmeIdentity {
    pub(super) model: Option<String>,
    pub(super) serial: Option<String>,
    pub(super) firmware: Option<String>,
    pub(super) standard: Option<String>,
    pub(super) features: Vec<String>,
}

pub(super) struct NvmeHealth {
    pub(super) temperature_celsius: Option<i16>,
    pub(super) life_remaining_percent: Option<u8>,
    pub(super) total_host_reads_bytes: u128,
    pub(super) total_host_writes_bytes: u128,
    pub(super) power_cycle_count: u128,
    pub(super) power_on_hours: u128,
    pub(super) health_state: DriveHealthState,
}

pub(super) fn parse_identify(data: &[u8]) -> Result<NvmeIdentity, String> {
    if data.len() < 4096 || read_u16(data, 0)? == 0 {
        return Err("NVMe Identify Controller data is invalid".to_string());
    }
    let model = parse_ascii(data.get(24..64).unwrap());
    if model.is_none() {
        return Err("NVMe Identify Controller model is invalid".to_string());
    }

    let mut features = Vec::new();
    if read_u16(data, 520)? & (1 << 2) != 0 {
        features.push("TRIM".to_string());
    }
    if data[525] & 1 != 0 {
        features.push("VolatileWriteCache".to_string());
    }

    let version = read_u32(data, 80)?;
    let standard = if version == 0 {
        None
    } else {
        let major = version >> 16;
        let minor = (version >> 8) & 0xFF;
        let tertiary = version & 0xFF;
        Some(if tertiary == 0 {
            format!("NVMe {major}.{minor}")
        } else {
            format!("NVMe {major}.{minor}.{tertiary}")
        })
    };

    Ok(NvmeIdentity {
        serial: parse_ascii(data.get(4..24).unwrap()),
        model,
        firmware: parse_ascii(data.get(64..72).unwrap()),
        standard,
        features,
    })
}

pub(super) fn parse_health(data: &[u8]) -> Result<NvmeHealth, String> {
    if data.len() < 512 || data[..512].iter().all(|byte| *byte == 0 || *byte == 0xFF) {
        return Err("NVMe SMART log is invalid".to_string());
    }
    let critical_warning = data[0];
    let kelvin = read_u16(data, 1)?;
    let temperature_celsius = if kelvin == 0 {
        None
    } else {
        i16::try_from(i32::from(kelvin) - 273).ok()
    };
    let available_spare = data[3];
    let spare_threshold = data[4];
    let percentage_used = data[5];
    let life_remaining_percent = Some(100u8.saturating_sub(percentage_used));
    let valid_spare = available_spare <= 100 && spare_threshold <= 100 && spare_threshold > 0;
    let health_state =
        if critical_warning != 0 || (valid_spare && available_spare < spare_threshold) {
            DriveHealthState::Critical
        } else if (valid_spare && available_spare == spare_threshold)
            || life_remaining_percent.is_some_and(|life| life <= 10)
        {
            DriveHealthState::Warning
        } else {
            DriveHealthState::Good
        };

    Ok(NvmeHealth {
        temperature_celsius,
        life_remaining_percent,
        total_host_reads_bytes: read_u128(data, 32)?.saturating_mul(512_000),
        total_host_writes_bytes: read_u128(data, 48)?.saturating_mul(512_000),
        power_cycle_count: read_u128(data, 112)?,
        power_on_hours: read_u128(data, 128)?,
        health_state,
    })
}

pub(super) fn rotation() -> DriveRotation {
    DriveRotation::SolidState
}

fn parse_ascii(bytes: &[u8]) -> Option<String> {
    if bytes
        .iter()
        .any(|byte| *byte != 0 && !byte.is_ascii_graphic() && *byte != b' ')
    {
        return None;
    }
    let value = std::str::from_utf8(bytes)
        .ok()?
        .trim_matches(|character| character == '\0' || character == ' ');
    (!value.is_empty()).then(|| value.to_string())
}

fn read_u16(data: &[u8], offset: usize) -> Result<u16, String> {
    let bytes = data
        .get(offset..offset + 2)
        .ok_or_else(|| "NVMe field is truncated".to_string())?;
    Ok(u16::from_le_bytes(bytes.try_into().unwrap()))
}

fn read_u32(data: &[u8], offset: usize) -> Result<u32, String> {
    let bytes = data
        .get(offset..offset + 4)
        .ok_or_else(|| "NVMe field is truncated".to_string())?;
    Ok(u32::from_le_bytes(bytes.try_into().unwrap()))
}

fn read_u128(data: &[u8], offset: usize) -> Result<u128, String> {
    let bytes = data
        .get(offset..offset + 16)
        .ok_or_else(|| "NVMe counter is truncated".to_string())?;
    Ok(u128::from_le_bytes(bytes.try_into().unwrap()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identify_uses_controller_fields_and_capabilities() {
        let mut data = [0u8; 4096];
        data[0..2].copy_from_slice(&0x1234u16.to_le_bytes());
        data[4..24].copy_from_slice(b"SN123               ");
        data[24..64].copy_from_slice(b"Example NVMe                            ");
        data[64..72].copy_from_slice(b"1.0     ");
        data[80..84].copy_from_slice(&0x0002_0100u32.to_le_bytes());
        data[520..522].copy_from_slice(&(1u16 << 2).to_le_bytes());
        data[525] = 1;

        let parsed = parse_identify(&data).unwrap();
        assert_eq!(parsed.model.as_deref(), Some("Example NVMe"));
        assert_eq!(parsed.serial.as_deref(), Some("SN123"));
        assert_eq!(parsed.standard.as_deref(), Some("NVMe 2.1"));
        assert_eq!(parsed.features, ["TRIM", "VolatileWriteCache"]);
    }

    #[test]
    fn smart_log_parses_counters_and_warning_state() {
        let mut data = [0u8; 512];
        data[1..3].copy_from_slice(&303u16.to_le_bytes());
        data[3] = 10;
        data[4] = 10;
        data[5] = 91;
        data[32..48].copy_from_slice(&2u128.to_le_bytes());
        data[48..64].copy_from_slice(&3u128.to_le_bytes());
        data[112..128].copy_from_slice(&4u128.to_le_bytes());
        data[128..144].copy_from_slice(&5u128.to_le_bytes());

        let parsed = parse_health(&data).unwrap();
        assert_eq!(parsed.temperature_celsius, Some(30));
        assert_eq!(parsed.life_remaining_percent, Some(9));
        assert_eq!(parsed.total_host_reads_bytes, 1_024_000);
        assert_eq!(parsed.total_host_writes_bytes, 1_536_000);
        assert_eq!(parsed.power_cycle_count, 4);
        assert_eq!(parsed.power_on_hours, 5);
        assert_eq!(parsed.health_state, DriveHealthState::Warning);
    }

    #[test]
    fn critical_warning_takes_precedence() {
        let mut data = [0u8; 512];
        data[0] = 1;
        assert_eq!(
            parse_health(&data).unwrap().health_state,
            DriveHealthState::Critical
        );
    }

    #[test]
    fn rejects_zero_filled_smart_log() {
        assert!(parse_health(&[0u8; 512]).is_err());
    }
}
