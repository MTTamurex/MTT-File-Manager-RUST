//! Formatting helpers for Windows
//! Follows .cursorrules: single responsibility, < 300 lines

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
        return "Desconhecido".to_string();
    }
    
    let seconds_in_day = 86400u64;
    let days_since_epoch = timestamp / seconds_in_day;
    let seconds_of_day = timestamp % seconds_in_day;

    let hour = (seconds_of_day / 3600) % 24;
    let minute = (seconds_of_day / 60) % 60;

    // Howard Hinnant's algorithm to convert days since epoch to y/m/d
    let z = (days_since_epoch as i64) + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe * 2000 + 1) / 730485;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let final_y = y + (if m <= 2 { 0 } else { 1 });

    let display_y = if m <= 2 { final_y + 1 } else { final_y };

    format!("{:02}/{:02}/{:04} {:02}:{:02}", d, m, display_y, hour, minute)
}
