//! Formatting helpers for Windows
//! Follows .cursorrules: single responsibility, < 300 lines
use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
use rust_i18n::t;

/// Formats bytes into human-readable size (KB, MB, GB).
pub fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} bytes", bytes)
    }
}

/// Formats Unix timestamp into DD/MM/YYYY HH:MM string.
pub fn format_date(timestamp: u64) -> String {
    if timestamp == 0 {
        return t!("file_info.unknown_date").to_string();
    }

    if let Some(local_time) = unix_timestamp_to_local_system_time(timestamp) {
        return format!(
            "{:02}/{:02}/{:04} {:02}:{:02}",
            local_time.wDay,
            local_time.wMonth,
            local_time.wYear,
            local_time.wHour,
            local_time.wMinute
        );
    }

    // Fallback if Windows local-time conversion fails for any reason.
    format_date_utc(timestamp)
}

fn unix_timestamp_to_local_system_time(timestamp: u64) -> Option<SYSTEMTIME> {
    const UNIX_TO_FILETIME_SECS: u64 = 11_644_473_600;
    const HUNDRED_NS_PER_SEC: u64 = 10_000_000;

    let filetime_ticks = timestamp
        .checked_add(UNIX_TO_FILETIME_SECS)?
        .checked_mul(HUNDRED_NS_PER_SEC)?;

    let file_time = FILETIME {
        dwLowDateTime: filetime_ticks as u32,
        dwHighDateTime: (filetime_ticks >> 32) as u32,
    };

    let mut utc_system_time = SYSTEMTIME::default();
    unsafe {
        FileTimeToSystemTime(&file_time, &mut utc_system_time).ok()?;
    }

    let mut local_system_time = SYSTEMTIME::default();
    let local_ok =
        unsafe { SystemTimeToTzSpecificLocalTime(None, &utc_system_time, &mut local_system_time) }
            .is_ok();

    if local_ok {
        Some(local_system_time)
    } else {
        Some(utc_system_time)
    }
}

fn format_date_utc(timestamp: u64) -> String {
    const SECONDS_IN_DAY: u64 = 86_400;
    let days_since_epoch = (timestamp / SECONDS_IN_DAY) as i64;
    let seconds_of_day = timestamp % SECONDS_IN_DAY;

    let hour = (seconds_of_day / 3600) % 24;
    let minute = (seconds_of_day / 60) % 60;

    // Howard Hinnant civil_from_days algorithm (Unix epoch days -> Y/M/D)
    let z = days_since_epoch + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = z - era * 146097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let mut y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    y += if m <= 2 { 1 } else { 0 };

    format!("{:02}/{:02}/{:04} {:02}:{:02}", d, m, y, hour, minute)
}

/// Formats duration in 100-nanosecond units to HH:MM:SS or MM:SS.
pub fn format_media_duration(duration_100ns: u64) -> String {
    let total_seconds = duration_100ns / 10_000_000;
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;

    if hours > 0 {
        format!("{:02}:{:02}:{:02}", hours, minutes, seconds)
    } else {
        format!("{:02}:{:02}", minutes, seconds)
    }
}

/// Formats bitrate in bits per second to bps, Kbps, or Mbps.
pub fn format_bitrate(bps: u32) -> String {
    let bps = bps as f64;
    if bps >= 1_000_000.0 {
        format!("{:.1} Mbps", bps / 1_000_000.0)
    } else if bps >= 1_000.0 {
        format!("{:.0} Kbps", bps / 1_000.0)
    } else {
        format!("{:.0} bps", bps)
    }
}

/// Approximates bitrate from file size and duration.
pub fn approximate_bitrate(size_bytes: u64, duration_100ns: u64) -> Option<u32> {
    if duration_100ns == 0 {
        return None;
    }
    let seconds = duration_100ns as f64 / 10_000_000.0;
    if seconds <= 0.0 {
        return None;
    }
    let bits_per_sec = (size_bytes as f64 * 8.0) / seconds;
    Some(bits_per_sec.max(0.0) as u32)
}

#[cfg(test)]
mod tests {
    use super::{format_date, format_date_utc};

    #[test]
    fn format_date_utc_handles_known_dates() {
        assert_eq!(format_date_utc(1760097600), "10/10/2025 12:00");
        assert_eq!(format_date_utc(1770638400), "09/02/2026 12:00");
        assert_eq!(format_date_utc(1770465600), "07/02/2026 12:00");
    }

    #[test]
    fn format_date_utc_handles_leap_day() {
        assert_eq!(format_date_utc(1709221500), "29/02/2024 15:45");
    }

    #[test]
    fn format_date_zero_is_unknown() {
        assert_eq!(format_date(0), "Desconhecido");
    }

    #[test]
    fn format_date_returns_expected_shape() {
        let s = format_date(1760097600);
        assert_eq!(s.len(), 16);
        assert_eq!(&s[2..3], "/");
        assert_eq!(&s[5..6], "/");
        assert_eq!(&s[10..11], " ");
        assert_eq!(&s[13..14], ":");
    }
}
