/// Format time in seconds to HH:MM:SS or MM:SS
pub fn format_time(seconds: f64) -> String {
    let total = seconds.max(0.0).floor() as i64;
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{:01}:{:02}:{:02}", h, m, s)
    } else {
        format!("{:02}:{:02}", m, s)
    }
}
