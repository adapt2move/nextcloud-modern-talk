//! One cached system timezone. Discovery runs on the identity worker; UI-facing
//! formatting and deadline resolution only clone this bounded memory snapshot.
use jiff::{
    Timestamp, ToSpan,
    civil::DateTime,
    tz::{AmbiguousOffset, TimeZone},
};
use std::sync::{OnceLock, RwLock};
use talk_core::{
    bounded::Text,
    preferences::time::{DeadlineChoice, DeadlineMoment, LocalTimeError, RepeatedTime},
};

static SYSTEM_ZONE: OnceLock<RwLock<Option<TimeZone>>> = OnceLock::new();

/// Call initially and at most once a minute on the identity worker. A transient
/// discovery failure preserves the last known zone; no UTC guess is exposed.
pub(crate) fn refresh_system_zone() {
    if let Ok(zone) = TimeZone::try_system() {
        *SYSTEM_ZONE
            .get_or_init(|| RwLock::new(None))
            .write()
            .unwrap_or_else(|e| e.into_inner()) = Some(zone);
    }
}

fn zone() -> Result<TimeZone, LocalTimeError> {
    SYSTEM_ZONE
        .get_or_init(|| RwLock::new(None))
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .ok_or(LocalTimeError::Unavailable)
}

pub fn time_label(seconds: u64) -> Option<String> {
    let local = timestamp(seconds).ok()?.to_zoned(zone().ok()?);
    Some(format!("{:02}:{:02}", local.hour(), local.minute()))
}

pub fn local_datetime(seconds: u64) -> Option<Text<32>> {
    datetime_in(seconds, &zone().ok()?).ok()
}

pub fn same_local_day(first: u64, second: u64) -> bool {
    let Ok(zone) = zone() else {
        return false;
    };
    same_day_in(&zone, first, second)
}

fn same_day_in(zone: &TimeZone, first: u64, second: u64) -> bool {
    let (Ok(first), Ok(second)) = (timestamp(first), timestamp(second)) else {
        return false;
    };
    first.to_zoned(zone.clone()).date() == second.to_zoned(zone.clone()).date()
}

/// The wall clock is read here, outside presentation. Timezone discovery never
/// happens on this path; custom wall times reject DST gaps and require an
/// explicit occurrence for repeated clock times.
pub fn deadline(choice: DeadlineChoice) -> Result<DeadlineMoment, LocalTimeError> {
    resolve_in(&zone()?, Timestamp::now(), choice)
}

fn timestamp(seconds: u64) -> Result<Timestamp, LocalTimeError> {
    Timestamp::from_second(i64::try_from(seconds).map_err(|_| LocalTimeError::Invalid)?)
        .map_err(|_| LocalTimeError::Invalid)
}

fn datetime_in(seconds: u64, zone: &TimeZone) -> Result<Text<32>, LocalTimeError> {
    let local = timestamp(seconds)?.to_zoned(zone.clone());
    Text::new(&format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        local.year(),
        local.month(),
        local.day(),
        local.hour(),
        local.minute()
    ))
    .map_err(|_| LocalTimeError::Invalid)
}

fn resolve_in(
    zone: &TimeZone,
    now: Timestamp,
    choice: DeadlineChoice,
) -> Result<DeadlineMoment, LocalTimeError> {
    let at = match choice {
        DeadlineChoice::InThirtyMinutes => now
            .checked_add(30.minutes())
            .map_err(|_| LocalTimeError::Invalid)?,
        DeadlineChoice::InOneHour => now
            .checked_add(1.hours())
            .map_err(|_| LocalTimeError::Invalid)?,
        DeadlineChoice::TomorrowMorning | DeadlineChoice::NextMondayMorning => {
            let date = now.to_zoned(zone.clone()).date();
            let days = if matches!(choice, DeadlineChoice::TomorrowMorning) {
                1
            } else {
                7 - i64::from(date.weekday().to_monday_zero_offset())
            };
            let tomorrow = date
                .checked_add(days.days())
                .map_err(|_| LocalTimeError::Invalid)?;
            resolve_wall(zone, tomorrow.at(9, 0, 0, 0), RepeatedTime::Ask)?
        }
        DeadlineChoice::Custom { local, repeated } => {
            let value = local.as_str();
            if value.len() != 16 || value.as_bytes().get(10) != Some(&b' ') {
                return Err(LocalTimeError::Invalid);
            }
            let mut normalized = value.to_owned();
            normalized.replace_range(10..11, "T");
            let datetime: DateTime = normalized.parse().map_err(|_| LocalTimeError::Invalid)?;
            if !(1970..=2100).contains(&datetime.year()) {
                return Err(LocalTimeError::Invalid);
            }
            resolve_wall(zone, datetime, repeated)?
        }
    };
    if at <= now {
        return Err(LocalTimeError::Past);
    }
    let timestamp = u64::try_from(at.as_second()).map_err(|_| LocalTimeError::Invalid)?;
    Ok(DeadlineMoment {
        timestamp,
        local: datetime_in(timestamp, zone)?,
    })
}

fn resolve_wall(
    zone: &TimeZone,
    local: DateTime,
    repeated: RepeatedTime,
) -> Result<Timestamp, LocalTimeError> {
    let ambiguous = zone.to_ambiguous_timestamp(local);
    match ambiguous.offset() {
        AmbiguousOffset::Unambiguous { .. } => {
            ambiguous.unambiguous().map_err(|_| LocalTimeError::Invalid)
        }
        AmbiguousOffset::Gap { .. } => Err(LocalTimeError::Skipped),
        AmbiguousOffset::Fold { .. } => match repeated {
            RepeatedTime::Ask => Err(LocalTimeError::Repeated),
            RepeatedTime::First => ambiguous.earlier().map_err(|_| LocalTimeError::Invalid),
            RepeatedTime::Second => ambiguous.later().map_err(|_| LocalTimeError::Invalid),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn custom(value: &str, repeated: RepeatedTime) -> DeadlineChoice {
        DeadlineChoice::Custom {
            local: Text::new(value).unwrap(),
            repeated,
        }
    }
    #[test]
    fn local_system_clock_format_uses_the_given_zone_without_utc_suffix() {
        let berlin = TimeZone::get("Europe/Berlin").unwrap();
        let utc: Timestamp = "2026-10-23T09:15:00Z".parse().unwrap();
        assert_eq!(
            datetime_in(utc.as_second() as u64, &berlin)
                .unwrap()
                .as_str(),
            "2026-10-23 11:15"
        );
        let winter: Timestamp = "2026-12-23T09:15:00Z".parse().unwrap();
        assert_eq!(
            datetime_in(winter.as_second() as u64, &berlin)
                .unwrap()
                .as_str(),
            "2026-12-23 10:15"
        );
        assert!(datetime_in(u64::MAX, &berlin).is_err());
        let seconds = |value: &str| value.parse::<Timestamp>().unwrap().as_second() as u64;
        assert!(same_day_in(
            &berlin,
            seconds("2026-10-23T23:59:00Z"),
            seconds("2026-10-24T00:00:00Z")
        ));
        assert!(!same_day_in(
            &berlin,
            seconds("2026-10-23T21:59:00Z"),
            seconds("2026-10-23T22:00:00Z")
        ));
    }
    #[test]
    fn custom_dates_require_explicit_repeated_time_and_reject_skipped_or_past_times() {
        let berlin = TimeZone::get("Europe/Berlin").unwrap();
        let now: Timestamp = "2026-01-01T00:00:00Z".parse().unwrap();
        assert_eq!(
            resolve_in(&berlin, now, custom("2026-03-29 02:30", RepeatedTime::Ask)),
            Err(LocalTimeError::Skipped)
        );
        assert_eq!(
            resolve_in(&berlin, now, custom("2026-10-25 02:30", RepeatedTime::Ask)),
            Err(LocalTimeError::Repeated)
        );
        let first = resolve_in(
            &berlin,
            now,
            custom("2026-10-25 02:30", RepeatedTime::First),
        )
        .unwrap();
        let second = resolve_in(
            &berlin,
            now,
            custom("2026-10-25 02:30", RepeatedTime::Second),
        )
        .unwrap();
        assert_eq!(first.timestamp + 3600, second.timestamp);
        for bad in [
            "2026-02-29 09:00",
            "2026-01-02 25:00",
            "2026-01-02 09:00 +01:00",
            "not a date",
        ] {
            assert_eq!(
                resolve_in(&berlin, now, custom(bad, RepeatedTime::Ask)),
                Err(LocalTimeError::Invalid)
            );
        }
        assert_eq!(
            resolve_in(&berlin, now, custom("2025-12-31 09:00", RepeatedTime::Ask)),
            Err(LocalTimeError::Past)
        );
        assert!(resolve_in(&berlin, now, custom("2028-02-29 09:00", RepeatedTime::Ask)).is_ok());
    }
    #[test]
    fn presets_use_elapsed_minutes_and_local_calendar_days_across_dst() {
        let berlin = TimeZone::get("Europe/Berlin").unwrap();
        let now: Timestamp = "2026-03-28T08:00:00Z".parse().unwrap();
        let tomorrow = resolve_in(&berlin, now, DeadlineChoice::TomorrowMorning).unwrap();
        assert_eq!(tomorrow.local.as_str(), "2026-03-29 09:00");
        assert_eq!(tomorrow.timestamp - now.as_second() as u64, 23 * 3600);
        let hour = resolve_in(&berlin, now, DeadlineChoice::InOneHour).unwrap();
        assert_eq!(hour.timestamp - now.as_second() as u64, 3600);
        let monday = resolve_in(&berlin, now, DeadlineChoice::NextMondayMorning).unwrap();
        assert_eq!(monday.local.as_str(), "2026-03-30 09:00");
    }
}
