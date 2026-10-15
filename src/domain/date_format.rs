use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum DateFormat {
    #[default]
    DayMonthYear,
    MonthDayYear,
    YearMonthDay,
}

static DATE_FORMAT: AtomicU8 = AtomicU8::new(DateFormat::DayMonthYear as u8);

pub fn date_format() -> DateFormat {
    match DATE_FORMAT.load(Ordering::Relaxed) {
        1 => DateFormat::MonthDayYear,
        2 => DateFormat::YearMonthDay,
        _ => DateFormat::DayMonthYear,
    }
}

pub fn set_date_format(format: DateFormat) {
    DATE_FORMAT.store(format as u8, Ordering::Relaxed);
}

impl DateFormat {
    pub fn from_preference(value: &str) -> Self {
        match value {
            "mdy" => Self::MonthDayYear,
            "ymd" => Self::YearMonthDay,
            _ => Self::DayMonthYear,
        }
    }

    pub fn preference_value(self) -> &'static str {
        match self {
            Self::DayMonthYear => "dmy",
            Self::MonthDayYear => "mdy",
            Self::YearMonthDay => "ymd",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Self::DayMonthYear => "DD/MM/YYYY",
            Self::MonthDayYear => "MM/DD/YYYY",
            Self::YearMonthDay => "YYYY-MM-DD",
        }
    }

    pub fn format(self, year: i64, month: i64, day: i64) -> String {
        match self {
            Self::DayMonthYear => format!("{day:02}/{month:02}/{year:04}"),
            Self::MonthDayYear => format!("{month:02}/{day:02}/{year:04}"),
            Self::YearMonthDay => format!("{year:04}-{month:02}-{day:02}"),
        }
    }

    pub fn separator_positions(self) -> [usize; 2] {
        if self == Self::YearMonthDay {
            [4, 6]
        } else {
            [2, 4]
        }
    }

    /// BIOSReleaseDate uses the firmware's month/day/year source order,
    /// independently of the Windows locale and the selected display format.
    pub fn reformat_bios_date(self, text: &str) -> Option<String> {
        let text = text.trim();
        if let Some(date) = self.reformat_year_first(text) {
            return Some(date);
        }
        let mut parts = text.split('/');
        let month = parts.next()?;
        let day = parts.next()?;
        let year = parts.next()?;
        if parts.next().is_some()
            || !(1..=2).contains(&month.len())
            || !(1..=2).contains(&day.len())
            || year.len() != 4
            || !month
                .bytes()
                .chain(day.bytes())
                .chain(year.bytes())
                .all(|b| b.is_ascii_digit())
        {
            return None;
        }
        let month: u32 = month.parse().ok()?;
        let day: u32 = day.parse().ok()?;
        self.reformat_year_first(&format!("{year}-{month:02}-{day:02}"))
    }

    /// Reformat unambiguous year-first metadata without changing its wall-clock time.
    /// ISO, EXIF and Shell property strings are source formats, independent of preferences.
    /// Unknown/localized strings are deliberately not guessed.
    pub fn reformat_year_first(self, text: &str) -> Option<String> {
        let date = text.get(..10)?;
        let separator = *date.as_bytes().get(4)?;
        if !matches!(separator, b'-' | b':' | b'/') || date.as_bytes()[7] != separator {
            return None;
        }
        let year: i64 = date.get(..4)?.parse().ok()?;
        let month: i64 = date.get(5..7)?.parse().ok()?;
        let day: i64 = date.get(8..10)?.parse().ok()?;
        let max_day = match month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
            2 => 28,
            _ => return None,
        };
        if !(1..=9999).contains(&year)
            || !(1..=max_day).contains(&day)
            || !date
                .bytes()
                .enumerate()
                .all(|(index, byte)| index == 4 || index == 7 || byte.is_ascii_digit())
        {
            return None;
        }
        let suffix = text.get(10..)?;
        if !suffix.is_empty() && !matches!(suffix.as_bytes()[0], b' ' | b'T' | b':') {
            return None;
        }
        let date = self.format(year, month, day);
        if suffix.is_empty() {
            Some(date)
        } else {
            Some(format!("{} {}", date, &suffix[1..]))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::DateFormat;

    #[test]
    fn bios_dates_follow_selected_format_without_reinterpreting_cached_source() {
        let cached = "03/04/2026".to_owned();
        assert_eq!(
            DateFormat::DayMonthYear
                .reformat_bios_date(&cached)
                .as_deref(),
            Some("04/03/2026")
        );
        assert_eq!(
            DateFormat::MonthDayYear
                .reformat_bios_date(&cached)
                .as_deref(),
            Some("03/04/2026")
        );
        assert_eq!(
            DateFormat::YearMonthDay
                .reformat_bios_date(&cached)
                .as_deref(),
            Some("2026-03-04")
        );
        assert_eq!(cached, "03/04/2026");
        assert_eq!(
            DateFormat::DayMonthYear
                .reformat_bios_date(" 2/29/2024 ")
                .as_deref(),
            Some("29/02/2024")
        );
        assert_eq!(
            DateFormat::MonthDayYear
                .reformat_bios_date("2024-02-29")
                .as_deref(),
            Some("02/29/2024")
        );
        for invalid in [
            "02/29/2023",
            "13/01/2026",
            "01/00/2026",
            "unknown",
            "",
            "03/04/26",
            "03/04/2026/05",
        ] {
            assert_eq!(DateFormat::DayMonthYear.reformat_bios_date(invalid), None);
        }
    }

    #[test]
    fn metadata_dates_preserve_time_and_use_explicit_source_order() {
        for source in [
            "2024-02-29 15:45:12",
            "2024:02:29 15:45:12",
            "2024/02/29:15:45:12",
        ] {
            assert_eq!(
                DateFormat::DayMonthYear
                    .reformat_year_first(source)
                    .as_deref(),
                Some("29/02/2024 15:45:12")
            );
            assert_eq!(
                DateFormat::MonthDayYear
                    .reformat_year_first(source)
                    .as_deref(),
                Some("02/29/2024 15:45:12")
            );
            assert_eq!(
                DateFormat::YearMonthDay
                    .reformat_year_first(source)
                    .as_deref(),
                Some("2024-02-29 15:45:12")
            );
        }
        for unknown in [
            "03/04/2026 12:00",
            "unknown",
            "2023-02-29",
            "2026-13-01",
            "2026-01-00",
            "0000-01-01",
            "图片日期",
        ] {
            assert_eq!(DateFormat::DayMonthYear.reformat_year_first(unknown), None);
        }
    }
}
