use super::*;

fn search_state() -> crate::app::global_search_state::GlobalSearchState {
    let (sender, _) = std::sync::mpsc::channel();
    let (_, receiver) = std::sync::mpsc::channel();
    crate::app::global_search_state::GlobalSearchState::new(sender, receiver)
}

#[test]
fn changing_format_preserves_actual_search_bounds() {
    let mut search = search_state();
    search.created_after_text = "03/02/2026".to_owned();
    search.created_before_text = "29/02/2024".to_owned();
    search.created_after_input_format = DateInputOrder::DayMonthYear;
    search.created_before_input_format = DateInputOrder::DayMonthYear;
    search.created_after = date_input_to_unix_ts(
        &search.created_after_text,
        search.created_after_input_format,
    );
    search.created_before = date_input_to_unix_ts_end_of_day(
        &search.created_before_text,
        search.created_before_input_format,
    );
    let original_bounds = (search.created_after, search.created_before);
    for format in [
        DateInputOrder::MonthDayYear,
        DateInputOrder::YearMonthDay,
        DateInputOrder::DayMonthYear,
    ] {
        reformat_search_dates(&mut search, format);
        assert_eq!(
            (search.created_after, search.created_before),
            original_bounds
        );
        assert_eq!(
            date_input_to_unix_ts(
                &search.created_after_text,
                search.created_after_input_format
            ),
            original_bounds.0
        );
        assert_eq!(
            date_input_to_unix_ts_end_of_day(
                &search.created_before_text,
                search.created_before_input_format
            ),
            original_bounds.1
        );
        assert_eq!(search.created_after_input_format, format);
        assert_eq!(search.created_before_input_format, format);
    }
}

#[test]
fn partial_edit_keeps_original_order_until_completed_or_emptied() {
    let mut text = "03/02".to_owned();
    let mut order = DateInputOrder::DayMonthYear;
    reformat_date_input(&mut text, &mut order, DateInputOrder::YearMonthDay);
    assert_eq!(text, "03/02");
    assert_eq!(order, DateInputOrder::DayMonthYear);
    text.push_str("/2026");
    let original = date_input_to_unix_ts(&text, order);
    reformat_date_input(&mut text, &mut order, DateInputOrder::YearMonthDay);
    assert_eq!(text, "2026-02-03");
    assert_eq!(date_input_to_unix_ts(&text, order), original);
    text.clear();
    reformat_date_input(&mut text, &mut order, DateInputOrder::MonthDayYear);
    assert_eq!(order, DateInputOrder::MonthDayYear);
}

#[test]
fn invalid_dates_remain_invalid_after_changing_format() {
    let mut text = "31/02/2026".to_owned();
    let mut order = DateInputOrder::DayMonthYear;
    reformat_date_input(&mut text, &mut order, DateInputOrder::YearMonthDay);
    assert_eq!(text, "2026-02-31");
    assert_eq!(date_input_to_unix_ts(&text, order), None);
}
