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

    let buf_len = buf_len.min(buffer.len());
    if buf_len < HEADER_SIZE {
        return None;
    }

    let num_identifiers = u32::from_le_bytes([buffer[8], buffer[9], buffer[10], buffer[11]]);
    let mut offset = HEADER_SIZE;
    let mut scsi_name_serial = None;

    for _ in 0..num_identifiers {
        if offset + IDENT_HEADER_SIZE > buf_len {
            break;
        }

        let code_set = i32::from_le_bytes(buffer[offset..offset + 4].try_into().ok()?);
        let ident_type = i32::from_le_bytes(buffer[offset + 4..offset + 8].try_into().ok()?);
        let ident_size =
            u16::from_le_bytes(buffer[offset + 8..offset + 10].try_into().ok()?) as usize;
        let next_offset =
            u16::from_le_bytes(buffer[offset + 10..offset + 12].try_into().ok()?) as usize;
        let data_start = offset + IDENT_HEADER_SIZE;
        let data_end = data_start.checked_add(ident_size)?;
        if data_end > buf_len {
            break;
        }

        let data = &buffer[data_start..data_end];
        if ident_type == TYPE_EUI64 && code_set == CODE_SET_BINARY && ident_size >= 8 {
            return Some(data[..8].iter().map(|byte| format!("{byte:02X}")).collect());
        }

        if ident_type == TYPE_SCSI_NAME_STRING
            && (code_set == CODE_SET_ASCII || code_set == CODE_SET_UTF8)
            && scsi_name_serial.is_none()
        {
            let end = data
                .iter()
                .position(|&byte| byte == 0)
                .unwrap_or(data.len());
            let value = String::from_utf8_lossy(&data[..end]).trim().to_string();
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
            break;
        }
        offset = offset.checked_add(next_offset)?;
    }

    scsi_name_serial
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
    use super::normalize_translated_nvme_identifier;

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
}
