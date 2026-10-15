use super::format_recycle_date;
use crate::domain::date_format::DateFormat;

#[test]
fn recycle_property_date_stays_canonical_for_live_format_changes() {
    let stored = format_recycle_date("2026/02/03:19:21:00.000");
    assert_eq!(stored, "2026-02-03 19:21");
    assert_eq!(
        DateFormat::DayMonthYear
            .reformat_year_first(&stored)
            .as_deref(),
        Some("03/02/2026 19:21")
    );
    assert_eq!(
        DateFormat::MonthDayYear
            .reformat_year_first(&stored)
            .as_deref(),
        Some("02/03/2026 19:21")
    );
    assert_eq!(
        DateFormat::YearMonthDay
            .reformat_year_first(&stored)
            .as_deref(),
        Some("2026-02-03 19:21")
    );
    assert_eq!(stored, "2026-02-03 19:21");
}
