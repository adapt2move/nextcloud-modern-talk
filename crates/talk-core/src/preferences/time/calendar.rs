//! Bounded civil calendar arithmetic. This never guesses a timezone; the host
//! resolves the selected local time and reports daylight-saving gaps and folds.
use crate::bounded::Text;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Calendar {
    year: u16,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
}

fn leap(year: u16) -> bool {
    year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
}

fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap(year) => 29,
        2 => 28,
        _ => 0,
    }
}

impl Calendar {
    pub fn parse(value: &str) -> Option<Self> {
        let bytes = value.as_bytes();
        if bytes.len() != 16
            || bytes[4] != b'-'
            || bytes[7] != b'-'
            || bytes[10] != b' '
            || bytes[13] != b':'
        {
            return None;
        }
        let number = |range: std::ops::Range<usize>| -> Option<u16> {
            bytes.get(range)?.iter().try_fold(0u16, |value, digit| {
                if !digit.is_ascii_digit() {
                    return None;
                }
                value.checked_mul(10)?.checked_add(u16::from(*digit - b'0'))
            })
        };
        let year = number(0..4)?;
        let month = u8::try_from(number(5..7)?).ok()?;
        let day = u8::try_from(number(8..10)?).ok()?;
        let hour = u8::try_from(number(11..13)?).ok()?;
        let minute = u8::try_from(number(14..16)?).ok()?;
        if !(1970..=2100).contains(&year)
            || day == 0
            || day > days_in_month(year, month)
            || hour > 23
            || minute > 59
        {
            return None;
        }
        Some(Self {
            year,
            month,
            day,
            hour,
            minute,
        })
    }

    pub fn local(self) -> Option<Text<32>> {
        Text::new(&format!(
            "{:04}-{:02}-{:02} {:02}:{:02}",
            self.year, self.month, self.day, self.hour, self.minute
        ))
        .ok()
    }

    pub fn year(self) -> u16 {
        self.year
    }
    pub fn month_number(self) -> u8 {
        self.month
    }
    pub fn day(self) -> u8 {
        self.day
    }
    pub fn hour(self) -> u8 {
        self.hour
    }
    pub fn minute(self) -> u8 {
        self.minute
    }

    /// Monday-first calendar; unused cells have no focus or action.
    pub fn grid(self) -> [Option<u8>; 42] {
        let y = i32::from(self.year) - 1;
        let before_year = 365 * y + y / 4 - y / 100 + y / 400;
        let epoch = 365 * 1969 + 1969 / 4 - 1969 / 100 + 1969 / 400;
        let before_month: i32 = (1..self.month)
            .map(|month| i32::from(days_in_month(self.year, month)))
            .sum();
        let first = ((before_year - epoch + before_month + 3) % 7) as usize;
        let mut cells = [None; 42];
        for day in 1..=days_in_month(self.year, self.month) {
            cells[first + usize::from(day) - 1] = Some(day);
        }
        cells
    }

    pub fn select(&mut self, day: u8) -> bool {
        if day == 0 || day > days_in_month(self.year, self.month) {
            return false;
        }
        self.day = day;
        true
    }

    pub fn month(&mut self, delta: i8) -> bool {
        if !matches!(delta, -1 | 1) {
            return false;
        }
        let index = i32::from(self.year) * 12 + i32::from(self.month) - 1 + i32::from(delta);
        let year = index.div_euclid(12);
        if !(1970..=2100).contains(&year) {
            return false;
        }
        self.year = year as u16;
        self.month = (index.rem_euclid(12) + 1) as u8;
        self.day = self.day.min(days_in_month(self.year, self.month));
        true
    }

    pub fn shift_days(&mut self, delta: i8) -> bool {
        if !(-7..=7).contains(&delta) {
            return false;
        }
        let mut next = *self;
        for _ in 0..delta.unsigned_abs() {
            if delta > 0 {
                if next.day < days_in_month(next.year, next.month) {
                    next.day += 1;
                } else {
                    if !next.month(1) {
                        return false;
                    }
                    next.day = 1;
                }
            } else if next.day > 1 {
                next.day -= 1;
            } else {
                if !next.month(-1) {
                    return false;
                }
                next.day = days_in_month(next.year, next.month);
            }
        }
        *self = next;
        true
    }

    pub fn shift_minutes(&mut self, delta: i16) -> bool {
        if !(-60..=60).contains(&delta) {
            return false;
        }
        let total = i16::from(self.hour) * 60 + i16::from(self.minute) + delta;
        let day = total.div_euclid(1440);
        let mut next = *self;
        if day != 0 && !next.shift_days(day as i8) {
            return false;
        }
        let clock = total.rem_euclid(1440);
        next.hour = (clock / 60) as u8;
        next.minute = (clock % 60) as u8;
        *self = next;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_validates_and_places_monday_first_days() {
        let epoch = Calendar::parse("1970-01-01 00:00").unwrap();
        assert_eq!(epoch.grid()[3], Some(1));
        let leap = Calendar::parse("2024-02-29 23:58").unwrap();
        assert_eq!(leap.grid()[3], Some(1));
        assert_eq!(leap.grid().iter().flatten().count(), 29);
        for invalid in [
            "2023-02-29 09:00",
            "2024-00-01 09:00",
            "2024-01-01 24:00",
            "1969-12-31 23:59",
            "2024-01-01T09:00",
            "2024-01-01 09:６",
        ] {
            assert!(Calendar::parse(invalid).is_none());
        }
    }

    #[test]
    fn month_navigation_clamps_and_clock_steps_across_day_boundaries() {
        let mut value = Calendar::parse("2024-01-31 23:58").unwrap();
        assert!(value.month(1));
        assert_eq!(value.local().unwrap().as_str(), "2024-02-29 23:58");
        assert!(value.shift_minutes(5));
        assert_eq!(value.local().unwrap().as_str(), "2024-03-01 00:03");
        assert!(value.shift_days(-1));
        assert_eq!(value.local().unwrap().as_str(), "2024-02-29 00:03");
        let mut first = Calendar::parse("1970-01-01 00:00").unwrap();
        assert!(!first.shift_minutes(-1));
        assert_eq!(first.local().unwrap().as_str(), "1970-01-01 00:00");
    }
}
