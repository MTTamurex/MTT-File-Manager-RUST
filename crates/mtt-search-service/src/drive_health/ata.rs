use std::collections::HashMap;

use mtt_search_protocol::{DriveHealthState, DriveRotation};

pub(super) struct AtaIdentity {
    pub(super) model: Option<String>,
    pub(super) serial: Option<String>,
    pub(super) firmware: Option<String>,
    pub(super) rotation: DriveRotation,
    pub(super) current_transfer_mode: Option<String>,
    pub(super) max_transfer_mode: Option<String>,
    pub(super) standard: Option<String>,
    pub(super) features: Vec<String>,
}

pub(super) struct AtaHealth {
    pub(super) temperature_celsius: Option<i16>,
    pub(super) power_cycle_count: Option<u128>,
    pub(super) power_on_hours: Option<u128>,
    pub(super) health_state: DriveHealthState,
}

#[derive(Clone, Copy)]
struct SmartAttribute {
    id: u8,
    pre_failure: bool,
    current: u8,
    raw: u64,
}

pub(super) fn parse_identify(data: &[u8]) -> Result<AtaIdentity, String> {
    if data.len() < 512 || data.iter().all(|byte| *byte == 0 || *byte == 0xFF) {
        return Err("ATA IDENTIFY data is invalid".to_string());
    }
    let model = parse_word_swapped_string(data, 27, 20);
    if model.is_none() {
        return Err("ATA IDENTIFY model is invalid".to_string());
    }

    let word76 = word(data, 76)?;
    let word77 = word(data, 77)?;
    let word82 = word(data, 82)?;
    let word83 = word(data, 83)?;
    let word88 = word(data, 88)?;
    let word169 = word(data, 169)?;
    let mut features = Vec::new();
    if word82 != 0xFFFF && word82 & 1 != 0 {
        features.push("SMART".to_string());
    }
    if word169 != 0xFFFF && word169 & 1 != 0 {
        features.push("TRIM".to_string());
    }
    if word76 != 0xFFFF && word76 & (1 << 8) != 0 {
        features.push("NCQ".to_string());
    }
    if word83 != 0xFFFF && word83 & (1 << 3) != 0 {
        features.push("APM".to_string());
    }
    if word83 != 0xFFFF && word83 & (1 << 9) != 0 {
        features.push("AAM".to_string());
    }

    let (current_transfer_mode, max_transfer_mode) =
        transfer_modes(word(data, 63)?, word76, word77, word88);

    Ok(AtaIdentity {
        serial: parse_word_swapped_string(data, 10, 10),
        firmware: parse_word_swapped_string(data, 23, 4),
        model,
        rotation: rotation(word(data, 217)?),
        current_transfer_mode,
        max_transfer_mode,
        standard: ata_standard(word(data, 80)?),
        features,
    })
}

pub(super) fn parse_smart(
    data: &[u8],
    thresholds: Option<&[u8]>,
    rotation: &DriveRotation,
) -> Result<AtaHealth, String> {
    let attributes = parse_attributes(data)?;
    let thresholds = thresholds.and_then(parse_thresholds);
    let mut evaluated_threshold = false;
    let mut critical = false;
    let mut threshold_warning = false;
    if let Some(thresholds) = &thresholds {
        for attribute in &attributes {
            let Some(threshold) = thresholds
                .get(&attribute.id)
                .copied()
                .filter(|threshold| *threshold > 0)
            else {
                continue;
            };
            evaluated_threshold = true;
            if attribute.current <= threshold {
                if attribute.pre_failure {
                    critical = true;
                } else {
                    threshold_warning = true;
                }
            }
        }
    }
    let sector_warning = matches!(rotation, DriveRotation::Rpm(_))
        && attributes
            .iter()
            .any(|attribute| matches!(attribute.id, 0x05 | 0xC5 | 0xC6) && attribute.raw > 0);

    let temperature_celsius = attribute(&attributes, 0xC2)
        .and_then(plausible_temperature)
        .or_else(|| attribute(&attributes, 0xBE).and_then(plausible_temperature));

    Ok(AtaHealth {
        temperature_celsius,
        power_cycle_count: attribute(&attributes, 0x0C).map(|attribute| attribute.raw as u128),
        // Attribute 09's raw unit is vendor-specific and cannot be assumed to be hours.
        power_on_hours: None,
        health_state: if critical {
            DriveHealthState::Critical
        } else if threshold_warning || sector_warning {
            DriveHealthState::Warning
        } else if evaluated_threshold {
            DriveHealthState::Good
        } else {
            DriveHealthState::Unknown
        },
    })
}

fn parse_attributes(data: &[u8]) -> Result<Vec<SmartAttribute>, String> {
    if data.len() < 512
        || data[..512].iter().all(|byte| *byte == 0xFF)
        || data[..512]
            .iter()
            .fold(0u8, |sum, byte| sum.wrapping_add(*byte))
            != 0
    {
        return Err("ATA SMART data checksum is invalid".to_string());
    }
    let mut attributes = Vec::new();
    for slot in 0..30 {
        let start = 2 + slot * 12;
        let record = &data[start..start + 12];
        if record[0] == 0 {
            continue;
        }
        let mut raw = [0u8; 8];
        raw[..6].copy_from_slice(&record[5..11]);
        attributes.push(SmartAttribute {
            id: record[0],
            pre_failure: u16::from_le_bytes([record[1], record[2]]) & 1 != 0,
            current: record[3],
            raw: u64::from_le_bytes(raw),
        });
    }
    if attributes.is_empty() {
        return Err("ATA SMART data has no attributes".to_string());
    }
    Ok(attributes)
}

fn parse_thresholds(data: &[u8]) -> Option<HashMap<u8, u8>> {
    if data.len() < 512 || data.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)) != 0 {
        return None;
    }
    let mut thresholds = HashMap::new();
    for slot in 0..30 {
        let start = 2 + slot * 12;
        let record = data.get(start..start + 12)?;
        if record[0] != 0 {
            thresholds.insert(record[0], record[1]);
        }
    }
    Some(thresholds)
}

fn attribute(attributes: &[SmartAttribute], id: u8) -> Option<SmartAttribute> {
    attributes
        .iter()
        .copied()
        .find(|attribute| attribute.id == id)
}

fn plausible_temperature(attribute: SmartAttribute) -> Option<i16> {
    let value = (attribute.raw & 0xFF) as i16;
    (1..=125).contains(&value).then_some(value)
}

fn rotation(value: u16) -> DriveRotation {
    match value {
        1 => DriveRotation::SolidState,
        0x0401..=0xFFFE => DriveRotation::Rpm(value),
        _ => DriveRotation::Unknown,
    }
}

fn ata_standard(major_versions: u16) -> Option<String> {
    let name = if major_versions & (1 << 13) != 0 {
        "ACS-6"
    } else if major_versions & (1 << 12) != 0 {
        "ACS-5"
    } else if major_versions & (1 << 11) != 0 {
        "ACS-4"
    } else if major_versions & (1 << 10) != 0 {
        "ACS-3"
    } else if major_versions & (1 << 9) != 0 {
        "ACS-2"
    } else if major_versions & (1 << 8) != 0 {
        "ATA8-ACS"
    } else if major_versions & (1 << 7) != 0 {
        "ATA/ATAPI-7"
    } else if major_versions & (1 << 6) != 0 {
        "ATA/ATAPI-6"
    } else if major_versions & (1 << 5) != 0 {
        "ATA/ATAPI-5"
    } else if major_versions & (1 << 4) != 0 {
        "ATA/ATAPI-4"
    } else {
        return None;
    };
    Some(name.to_string())
}

fn transfer_modes(
    multiword_dma: u16,
    sata_capabilities: u16,
    sata_additional: u16,
    ultra_dma: u16,
) -> (Option<String>, Option<String>) {
    let ultra_dma = (ultra_dma != 0xFFFF).then_some(ultra_dma);
    let multiword_dma = (multiword_dma != 0xFFFF).then_some(multiword_dma);
    let current = sata_mode(sata_additional)
        .or_else(|| {
            ultra_dma
                .and_then(|word| highest_set((word >> 8) as u8))
                .map(|mode| format!("UDMA {mode}"))
        })
        .or_else(|| {
            multiword_dma
                .and_then(|word| highest_set((word >> 8) as u8))
                .map(|mode| format!("MWDMA {mode}"))
        });
    let max_sata = if sata_capabilities != 0 && sata_capabilities != 0xFFFF {
        if sata_capabilities & (1 << 3) != 0 {
            Some("SATA 6.0 Gb/s".to_string())
        } else if sata_capabilities & (1 << 2) != 0 {
            Some("SATA 3.0 Gb/s".to_string())
        } else if sata_capabilities & (1 << 1) != 0 {
            Some("SATA 1.5 Gb/s".to_string())
        } else {
            None
        }
    } else {
        None
    };
    let maximum = max_sata
        .or_else(|| {
            ultra_dma
                .and_then(|word| highest_set(word as u8))
                .map(|mode| format!("UDMA {mode}"))
        })
        .or_else(|| {
            multiword_dma
                .and_then(|word| highest_set(word as u8))
                .map(|mode| format!("MWDMA {mode}"))
        });
    (current, maximum)
}

fn sata_mode(word: u16) -> Option<String> {
    if word == 0 || word == 0xFFFF {
        return None;
    }
    match (word >> 1) & 0x7 {
        1 => Some("SATA 1.5 Gb/s".to_string()),
        2 => Some("SATA 3.0 Gb/s".to_string()),
        3 => Some("SATA 6.0 Gb/s".to_string()),
        _ => None,
    }
}

fn highest_set(bits: u8) -> Option<u32> {
    (bits != 0).then(|| 7 - bits.leading_zeros())
}

fn parse_word_swapped_string(data: &[u8], first_word: usize, words: usize) -> Option<String> {
    let start = first_word.checked_mul(2)?;
    let bytes = data.get(start..start.checked_add(words.checked_mul(2)?)?)?;
    let mut decoded = Vec::with_capacity(bytes.len());
    for word in bytes.chunks_exact(2) {
        decoded.extend_from_slice(&[word[1], word[0]]);
    }
    if decoded
        .iter()
        .any(|byte| *byte != 0 && !byte.is_ascii_graphic() && *byte != b' ')
    {
        return None;
    }
    let value = std::str::from_utf8(&decoded)
        .ok()?
        .trim_matches(|character| character == '\0' || character == ' ');
    (!value.is_empty()).then(|| value.to_string())
}

fn word(data: &[u8], index: usize) -> Result<u16, String> {
    let start = index
        .checked_mul(2)
        .ok_or_else(|| "ATA word offset overflow".to_string())?;
    let bytes = data
        .get(start..start + 2)
        .ok_or_else(|| "ATA IDENTIFY field is truncated".to_string())?;
    Ok(u16::from_le_bytes(bytes.try_into().unwrap()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set_swapped(data: &mut [u8], word_index: usize, value: &[u8]) {
        for (destination, source) in data[word_index * 2..]
            .chunks_exact_mut(2)
            .zip(value.chunks_exact(2))
        {
            destination.copy_from_slice(&[source[1], source[0]]);
        }
    }

    fn fix_checksum(data: &mut [u8; 512]) {
        let sum = data[..511]
            .iter()
            .fold(0u8, |sum, byte| sum.wrapping_add(*byte));
        data[511] = 0u8.wrapping_sub(sum);
    }

    #[test]
    fn identify_parses_word_swapped_fields_rotation_and_features() {
        let mut data = [0u8; 512];
        set_swapped(&mut data, 10, b"SERIAL1234567890    ");
        set_swapped(&mut data, 23, b"FW1     ");
        set_swapped(&mut data, 27, b"Example SATA SSD                        ");
        data[160..162].copy_from_slice(&(1u16 << 11).to_le_bytes());
        data[152..154].copy_from_slice(&((1u16 << 8) | (1 << 3)).to_le_bytes());
        data[154..156].copy_from_slice(&0u16.to_le_bytes());
        data[164..166].copy_from_slice(&1u16.to_le_bytes());
        data[166..168].copy_from_slice(&((1u16 << 3) | (1 << 9)).to_le_bytes());
        data[176..178].copy_from_slice(&((1u16 << 14) | (1 << 6)).to_le_bytes());
        data[338..340].copy_from_slice(&1u16.to_le_bytes());
        data[434..436].copy_from_slice(&1u16.to_le_bytes());

        let parsed = parse_identify(&data).unwrap();
        assert_eq!(parsed.model.as_deref(), Some("Example SATA SSD"));
        assert_eq!(parsed.rotation, DriveRotation::SolidState);
        assert_eq!(parsed.standard.as_deref(), Some("ACS-4"));
        assert!(parsed.features.contains(&"SMART".to_string()));
        assert!(parsed.features.contains(&"TRIM".to_string()));
        assert!(parsed.features.contains(&"NCQ".to_string()));
        assert_eq!(parsed.current_transfer_mode.as_deref(), Some("UDMA 6"));
        assert_eq!(parsed.max_transfer_mode.as_deref(), Some("SATA 6.0 Gb/s"));
    }

    #[test]
    fn smart_threshold_failure_is_critical() {
        let mut smart = [0u8; 512];
        smart[2] = 0x05;
        smart[3] = 1;
        smart[5] = 9;
        fix_checksum(&mut smart);
        let mut thresholds = [0u8; 512];
        thresholds[2] = 0x05;
        thresholds[3] = 10;
        fix_checksum(&mut thresholds);

        let parsed = parse_smart(&smart, Some(&thresholds), &DriveRotation::Rpm(7200)).unwrap();
        assert_eq!(parsed.health_state, DriveHealthState::Critical);
    }

    #[test]
    fn smart_threshold_equality_is_failure_and_advisory_is_warning() {
        let mut smart = [0u8; 512];
        smart[2] = 0x05;
        smart[3] = 1;
        smart[5] = 10;
        fix_checksum(&mut smart);
        let mut thresholds = [0u8; 512];
        thresholds[2] = 0x05;
        thresholds[3] = 10;
        fix_checksum(&mut thresholds);
        assert_eq!(
            parse_smart(&smart, Some(&thresholds), &DriveRotation::Unknown)
                .unwrap()
                .health_state,
            DriveHealthState::Critical
        );

        smart[3] = 0;
        fix_checksum(&mut smart);
        assert_eq!(
            parse_smart(&smart, Some(&thresholds), &DriveRotation::Unknown)
                .unwrap()
                .health_state,
            DriveHealthState::Warning
        );
    }

    #[test]
    fn smart_without_threshold_evidence_has_unknown_health() {
        let mut smart = [0u8; 512];
        smart[2] = 0x09;
        smart[5] = 100;
        smart[7] = 42;
        fix_checksum(&mut smart);

        let parsed = parse_smart(&smart, None, &DriveRotation::Unknown).unwrap();
        assert_eq!(parsed.power_on_hours, None);
        assert_eq!(parsed.health_state, DriveHealthState::Unknown);
    }

    #[test]
    fn sata_current_generation_is_an_enumerated_field() {
        assert_eq!(sata_mode(1 << 1).as_deref(), Some("SATA 1.5 Gb/s"));
        assert_eq!(sata_mode(2 << 1).as_deref(), Some("SATA 3.0 Gb/s"));
        assert_eq!(sata_mode(3 << 1).as_deref(), Some("SATA 6.0 Gb/s"));
        assert_eq!(sata_mode(0), None);
        assert_eq!(sata_mode(0xFFFF), None);
    }

    #[test]
    fn sata_maximum_does_not_depend_on_current_speed_word() {
        let (_, maximum) = transfer_modes(0, 1 << 3, 0xFFFF, 0);
        assert_eq!(maximum.as_deref(), Some("SATA 6.0 Gb/s"));
    }

    #[test]
    fn hdd_pending_sectors_are_warning_and_common_attributes_parse() {
        let mut smart = [0u8; 512];
        smart[2] = 0xC5;
        smart[5] = 100;
        smart[7] = 2;
        smart[14] = 0xC2;
        smart[17] = 100;
        smart[19] = 35;
        smart[26] = 0x09;
        smart[29] = 100;
        smart[31] = 42;
        fix_checksum(&mut smart);

        let parsed = parse_smart(&smart, None, &DriveRotation::Rpm(7200)).unwrap();
        assert_eq!(parsed.temperature_celsius, Some(35));
        assert_eq!(parsed.power_on_hours, None);
        assert_eq!(parsed.health_state, DriveHealthState::Warning);
    }

    #[test]
    fn rejects_smart_data_with_bad_checksum() {
        let mut smart = [0u8; 512];
        smart[2] = 0x09;
        assert!(parse_smart(&smart, None, &DriveRotation::Unknown).is_err());
    }

    #[test]
    fn rejects_all_ones_smart_data() {
        assert!(parse_smart(&[0xFF; 512], None, &DriveRotation::Unknown).is_err());
    }
}
