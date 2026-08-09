//! Parsing and normalization of Windows storage device identifiers.

/// Parses `STORAGE_DEVICE_ID_DESCRIPTOR` for an EUI-64 or SCSI name string.
pub(super) fn query_device_id_serial(buffer: &[u8], buf_len: usize) -> Option<String> {
    const HEADER_SIZE: usize = 12;
    const IDENT_HEADER_SIZE: usize = 16;
    const CODE_SET_BINARY: i32 = 1;
    const CODE_SET_ASCII: i32 = 2;
    const CODE_SET_UTF8: i32 = 3;
    const TYPE_EUI64: i32 = 2;
    const TYPE_SCSI_NAME_STRING: i32 = 8;
    const ASSOCIATION_DEVICE: i32 = 0;

    let buf_len = buf_len.min(buffer.len());
    if buf_len < HEADER_SIZE {
        return None;
    }

    let version = u32::from_le_bytes(buffer[0..4].try_into().ok()?) as usize;
    let descriptor_size = u32::from_le_bytes(buffer[4..8].try_into().ok()?) as usize;
    if version < HEADER_SIZE || version > descriptor_size || descriptor_size > buf_len {
        return None;
    }
    let buffer = &buffer[..descriptor_size];
    let num_identifiers = u32::from_le_bytes([buffer[8], buffer[9], buffer[10], buffer[11]]);
    if num_identifiers as usize > descriptor_size.saturating_sub(HEADER_SIZE) / IDENT_HEADER_SIZE {
        return None;
    }
    let mut offset = HEADER_SIZE;
    let mut eui64_serial = None;
    let mut scsi_name_serial = None;

    for index in 0..num_identifiers {
        let header_end = offset.checked_add(IDENT_HEADER_SIZE)?;
        if header_end > descriptor_size {
            return None;
        }

        let code_set = i32::from_le_bytes(buffer[offset..offset + 4].try_into().ok()?);
        let ident_type = i32::from_le_bytes(buffer[offset + 4..offset + 8].try_into().ok()?);
        let ident_size =
            u16::from_le_bytes(buffer[offset + 8..offset + 10].try_into().ok()?) as usize;
        let next_offset =
            u16::from_le_bytes(buffer[offset + 10..offset + 12].try_into().ok()?) as usize;
        let association = i32::from_le_bytes(buffer[offset + 12..offset + 16].try_into().ok()?);
        let data_start = offset + IDENT_HEADER_SIZE;
        let data_end = data_start.checked_add(ident_size)?;
        if data_end > descriptor_size {
            return None;
        }

        let data = &buffer[data_start..data_end];
        if association == ASSOCIATION_DEVICE
            && ident_type == TYPE_EUI64
            && code_set == CODE_SET_BINARY
            && ident_size == 8
            && eui64_serial.is_none()
        {
            eui64_serial = Some(data.iter().map(|byte| format!("{byte:02X}")).collect());
        }

        if association == ASSOCIATION_DEVICE
            && ident_type == TYPE_SCSI_NAME_STRING
            && (code_set == CODE_SET_ASCII || code_set == CODE_SET_UTF8)
            && scsi_name_serial.is_none()
        {
            let end = data
                .iter()
                .position(|&byte| byte == 0)
                .unwrap_or(data.len());
            if code_set == CODE_SET_ASCII && !data[..end].is_ascii() {
                return None;
            }
            let value = std::str::from_utf8(&data[..end]).ok()?.trim().to_string();
            if let Some(hex) = value.strip_prefix("eui.") {
                let cleaned = strip_leading_zero_pairs(hex);
                if !cleaned.is_empty() {
                    scsi_name_serial = Some(cleaned);
                }
            } else if !value.is_empty() {
                scsi_name_serial = Some(value);
            }
        }

        if next_offset == 0 {
            if index + 1 != num_identifiers {
                return None;
            }
        } else {
            if index + 1 == num_identifiers || next_offset < IDENT_HEADER_SIZE + ident_size {
                return None;
            }
            offset = offset.checked_add(next_offset)?;
            if offset > descriptor_size {
                return None;
            }
        }
    }

    eui64_serial.or(scsi_name_serial)
}

/// Kingston NVMe drives with OUI 0026B7 expose the controller's NAA serial as
/// a namespace EUI with the leading NAA-5 nibble moved to the end.
pub(super) fn normalize_translated_nvme_identifier(identifier: String) -> String {
    if identifier.len() == 16 && identifier.starts_with("0026B7") && identifier.ends_with('5') {
        format!("5{}", &identifier[..15])
    } else {
        identifier
    }
}

pub(super) fn clean_fallback_serial(serial: &str) -> String {
    serial.trim().replace('_', "")
}

fn strip_leading_zero_pairs(hex: &str) -> String {
    let mut result = hex.trim();
    while result.len() > 16 && result.starts_with("00") {
        result = &result[2..];
    }
    result.to_string()
}

#[cfg(test)]
mod tests {
    use super::{normalize_translated_nvme_identifier, query_device_id_serial};

    fn descriptor(identifiers: &[(i32, i32, i32, &[u8])]) -> Vec<u8> {
        let mut buffer = vec![0u8; 12];
        buffer[0..4].copy_from_slice(&16u32.to_le_bytes());
        buffer[8..12].copy_from_slice(&(identifiers.len() as u32).to_le_bytes());
        for (index, (code_set, ident_type, association, data)) in identifiers.iter().enumerate() {
            let record_size = 16 + data.len();
            buffer.extend_from_slice(&code_set.to_le_bytes());
            buffer.extend_from_slice(&ident_type.to_le_bytes());
            buffer.extend_from_slice(&(data.len() as u16).to_le_bytes());
            let next_offset = if index + 1 == identifiers.len() {
                0
            } else {
                record_size as u16
            };
            buffer.extend_from_slice(&next_offset.to_le_bytes());
            buffer.extend_from_slice(&association.to_le_bytes());
            buffer.extend_from_slice(data);
        }
        let size = buffer.len() as u32;
        buffer[4..8].copy_from_slice(&size.to_le_bytes());
        buffer
    }

    #[test]
    fn restores_kingston_nvme_naa_serial_from_translated_eui() {
        assert_eq!(
            normalize_translated_nvme_identifier("0026B77851D736D5".to_string()),
            "50026B77851D736D"
        );
    }

    #[test]
    fn leaves_unrecognized_identifiers_unchanged() {
        assert_eq!(
            normalize_translated_nvme_identifier("0123456789ABCDEF".to_string()),
            "0123456789ABCDEF"
        );
    }

    #[test]
    fn ignores_port_and_target_identifiers_and_accepts_device_identifier() {
        let buffer = descriptor(&[
            (1, 2, 1, &[0x11; 8]),
            (2, 8, 2, b"target-name"),
            (1, 2, 0, &[0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, 0x10, 0x20]),
        ]);

        assert_eq!(
            query_device_id_serial(&buffer, buffer.len()).as_deref(),
            Some("AABBCCDDEEFF1020")
        );

        let buffer = descriptor(&[(1, 2, 1, &[0x11; 8]), (2, 8, 2, b"target-name")]);
        assert_eq!(query_device_id_serial(&buffer, buffer.len()), None);
    }

    #[test]
    fn honors_descriptor_size_and_exact_eui64_length() {
        let mut outside_descriptor = descriptor(&[(1, 2, 0, &[0xAA; 8])]);
        outside_descriptor[4..8].copy_from_slice(&12u32.to_le_bytes());
        assert_eq!(
            query_device_id_serial(&outside_descriptor, outside_descriptor.len()),
            None
        );

        let wrong_size = descriptor(&[(1, 2, 0, &[0xAA; 9])]);
        assert_eq!(query_device_id_serial(&wrong_size, wrong_size.len()), None);
    }
}
