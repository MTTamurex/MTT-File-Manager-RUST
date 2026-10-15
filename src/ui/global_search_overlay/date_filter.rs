use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
use windows::Win32::System::Time::{SystemTimeToFileTime, TzSpecificLocalTimeToSystemTime};

pub(super) use crate::domain::date_format::DateFormat as DateInputOrder;

pub(crate) fn reformat_search_dates(
    search: &mut crate::app::global_search_state::GlobalSearchState,
    new: DateInputOrder,
) {
    reformat_date_input(
        &mut search.created_after_text,
        &mut search.created_after_input_format,
        new,
    );
    reformat_date_input(
        &mut search.created_before_text,
        &mut search.created_before_input_format,
        new,
    );
}

pub(super) fn reformat_date_input(
    text: &mut String,
    order: &mut DateInputOrder,
    new: DateInputOrder,
) {
    if *order == new {
        return;
    }
    if text.is_empty() {
        *order = new;
    } else if let Some((month, day, year)) = parse_date_input(text, *order) {
        *text = new.format(i64::from(year), i64::from(month), i64::from(day));
        *order = new;
    }
}

pub(super) fn format_date_input(input: &str, order: DateInputOrder) -> String {
    let digits: String = input.chars().filter(char::is_ascii_digit).take(8).collect();
    let mut formatted = String::with_capacity(10);

    for (index, digit) in digits.chars().enumerate() {
        if order.separator_positions().contains(&index) {
            formatted.push(if order == DateInputOrder::YearMonthDay {
                '-'
            } else {
                '/'
            });
        }
        formatted.push(digit);
    }

    formatted
}

pub(super) fn formatted_date_cursor_index(
    input: &str,
    cursor_index: usize,
    order: DateInputOrder,
) -> usize {
    let digits_before_cursor = input
        .chars()
        .take(cursor_index)
        .filter(char::is_ascii_digit)
        .take(8)
        .count();

    digits_before_cursor
        + order
            .separator_positions()
            .iter()
            .filter(|&&position| digits_before_cursor > position)
            .count()
}

pub(super) fn date_input_to_unix_ts(input: &str, order: DateInputOrder) -> Option<u64> {
    let (month, day, year) = parse_date_input(input, order)?;
    date_components_to_unix_ts(month, day, year)
}

pub(super) fn date_input_to_unix_ts_end_of_day(input: &str, order: DateInputOrder) -> Option<u64> {
    let (month, day, year) = parse_date_input(input, order)?;
    date_components_to_unix_ts_end_of_day(month, day, year)
}

pub(super) fn parse_date_input(input: &str, order: DateInputOrder) -> Option<(u32, u32, u32)> {
    let digits: String = input.chars().filter(char::is_ascii_digit).collect();
    if digits.len() != 8 {
        return None;
    }

    if order == DateInputOrder::YearMonthDay {
        return Some((
            digits[4..6].parse().ok()?,
            digits[6..8].parse().ok()?,
            digits[0..4].parse().ok()?,
        ));
    }
    let first = digits[0..2].parse().ok()?;
    let second = digits[2..4].parse().ok()?;
    let year = digits[4..8].parse().ok()?;
    let (month, day) = match order {
        DateInputOrder::DayMonthYear => (second, first),
        DateInputOrder::MonthDayYear => (first, second),
        DateInputOrder::YearMonthDay => unreachable!(),
    };

    Some((month, day, year))
}

/// Convert date components (month, day, year) to Unix timestamp at local midnight.
/// Returns `None` if any component is 0 (not set) or out of valid range.
fn date_components_to_unix_ts(month: u32, day: u32, year: u32) -> Option<u64> {
    validate_date_components(month, day, year)?;
    local_datetime_to_unix_ts(year, month, day, 0, 0, 0)
}

/// Convert date components to Unix timestamp at the end of the selected local day.
fn date_components_to_unix_ts_end_of_day(month: u32, day: u32, year: u32) -> Option<u64> {
    validate_date_components(month, day, year)?;
    let (next_year, next_month, next_day) = next_civil_date(year, month, day);
    local_datetime_to_unix_ts(next_year, next_month, next_day, 0, 0, 0)
        .map(|ts| ts.saturating_sub(1))
}

fn validate_date_components(month: u32, day: u32, year: u32) -> Option<()> {
    if month == 0 || day == 0 || year == 0 {
        return None;
    }
    if !(1..=12).contains(&month) || day > days_in_month(year, month) {
        return None;
    }
    if days_from_civil(year, month, day) < 0 {
        return None;
    }
    Some(())
}

fn local_datetime_to_unix_ts(
    year: u32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
) -> Option<u64> {
    const UNIX_TO_FILETIME_TICKS: u64 = 116_444_736_000_000_000;
    const HUNDRED_NS_PER_SEC: u64 = 10_000_000;

    let local_time = SYSTEMTIME {
        wYear: year.try_into().ok()?,
        wMonth: month.try_into().ok()?,
        wDayOfWeek: 0,
        wDay: day.try_into().ok()?,
        wHour: hour.try_into().ok()?,
        wMinute: minute.try_into().ok()?,
        wSecond: second.try_into().ok()?,
        wMilliseconds: 0,
    };

    let mut utc_time = SYSTEMTIME::default();
    unsafe {
        TzSpecificLocalTimeToSystemTime(None, &local_time, &mut utc_time).ok()?;
    }

    let mut file_time = FILETIME::default();
    unsafe {
        SystemTimeToFileTime(&utc_time, &mut file_time).ok()?;
    }

    let filetime_ticks = ((file_time.dwHighDateTime as u64) << 32) | file_time.dwLowDateTime as u64;
    if filetime_ticks <= UNIX_TO_FILETIME_TICKS {
        return Some(0);
    }

    Some((filetime_ticks - UNIX_TO_FILETIME_TICKS) / HUNDRED_NS_PER_SEC)
}

// Days from civil date to days since epoch (algorithm by Howard Hinnant).
fn days_from_civil(year: u32, month: u32, day: u32) -> i64 {
    let mut y = i64::from(year);
    let m = i64::from(month);
    let d = i64::from(day);
    y -= if m <= 2 { 1 } else { 0 };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = m + if m > 2 { -3 } else { 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn next_civil_date(year: u32, month: u32, day: u32) -> (u32, u32, u32) {
    if day < days_in_month(year, month) {
        return (year, month, day + 1);
    }
    if month < 12 {
        return (year, month + 1, 1);
    }
    (year + 1, 1, 1)
}

fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap_year(year: u32) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

#[cfg(test)]
#[path = "date_filter_review_tests.rs"]
mod review_tests;

#[cfg(test)]
mod tests {
    use super::{
        date_components_to_unix_ts, date_components_to_unix_ts_end_of_day, date_input_to_unix_ts,
        format_date_input, formatted_date_cursor_index, parse_date_input, DateInputOrder,
    };

    #[test]
    fn iso_mask_cursor_and_leap_day_match_the_selected_format() {
        let order = DateInputOrder::YearMonthDay;
        assert_eq!(format_date_input("202602", order), "2026-02");
        assert_eq!(format_date_input("20240229", order), "2024-02-29");
        assert_eq!(formatted_date_cursor_index("20240", 5, order), 6);
        assert_eq!(formatted_date_cursor_index("2024022", 7, order), 9);
        assert_eq!(parse_date_input("2024-02-29", order), Some((2, 29, 2024)));
        assert_eq!(date_input_to_unix_ts("2023-02-29", order), None);
        assert_eq!(
            date_input_to_unix_ts("2024-02-29", order),
            date_input_to_unix_ts("29/02/2024", DateInputOrder::DayMonthYear)
        );
    }

    #[test]
    fn switching_formats_preserves_ambiguous_dates_and_filter_bounds() {
        let formats = [
            DateInputOrder::DayMonthYear,
            DateInputOrder::MonthDayYear,
            DateInputOrder::YearMonthDay,
        ];
        for old in formats {
            let original = old.format(2026, 2, 3);
            let (month, day, year) = parse_date_input(&original, old).unwrap();
            for new in formats {
                let converted = new.format(i64::from(year), i64::from(month), i64::from(day));
                assert_eq!(
                    date_input_to_unix_ts(&original, old),
                    date_input_to_unix_ts(&converted, new)
                );
                assert_eq!(
                    super::date_input_to_unix_ts_end_of_day(&original, old),
                    super::date_input_to_unix_ts_end_of_day(&converted, new)
                );
            }
        }
    }

    #[test]
    fn date_input_formats_digits_as_a_single_field() {
        let order = DateInputOrder::DayMonthYear;
        assert_eq!(format_date_input("1", order), "1");
        assert_eq!(format_date_input("3112", order), "31/12");
        assert_eq!(format_date_input("31/12/2026", order), "31/12/2026");
        assert_eq!(format_date_input("31-12-2026-extra", order), "31/12/2026");
    }

    #[test]
    fn date_input_cursor_moves_past_inserted_separators() {
        let order = DateInputOrder::DayMonthYear;
        assert_eq!(formatted_date_cursor_index("12", 2, order), 2);
        assert_eq!(formatted_date_cursor_index("123", 3, order), 4);
        assert_eq!(formatted_date_cursor_index("12345", 5, order), 7);
        assert_eq!(formatted_date_cursor_index("12/3", 4, order), 4);
    }

    #[test]
    fn date_input_parses_localized_component_order() {
        assert_eq!(
            parse_date_input("31/12/2026", DateInputOrder::DayMonthYear),
            Some((12, 31, 2026))
        );
        assert_eq!(
            parse_date_input("12/31/2026", DateInputOrder::MonthDayYear),
            Some((12, 31, 2026))
        );
    }

    #[test]
    fn date_input_rejects_partial_or_invalid_dates() {
        assert_eq!(
            date_input_to_unix_ts("31/12", DateInputOrder::DayMonthYear),
            None
        );
        assert_eq!(
            date_input_to_unix_ts("31/02/2026", DateInputOrder::DayMonthYear),
            None
        );
    }

    #[test]
    fn date_components_reject_pre_epoch_dates() {
        assert_eq!(date_components_to_unix_ts(12, 31, 1969), None);
        assert!(date_components_to_unix_ts(1, 1, 1970).is_some());
    }

    #[test]
    fn date_components_reject_nonexistent_calendar_dates() {
        assert_eq!(date_components_to_unix_ts(2, 31, 2024), None);
        assert_eq!(date_components_to_unix_ts(4, 31, 2024), None);
        assert_eq!(date_components_to_unix_ts(2, 29, 2023), None);
        assert!(date_components_to_unix_ts(2, 29, 2024).is_some());
    }

    #[test]
    fn date_components_end_of_day_is_after_start() {
        let start = date_components_to_unix_ts(6, 11, 2026).unwrap();
        let end = date_components_to_unix_ts_end_of_day(6, 11, 2026).unwrap();
        assert!(end > start);
    }
}
