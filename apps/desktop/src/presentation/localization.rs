//! Own interface copy only. User, room and message content never enters this catalog.
//! Source keys are stable German author strings; translations remain centralized.
mod call;
mod chat;
mod connection;
mod content;
mod conversation;
mod files;
mod onboarding;
mod own_status;
mod people;
mod polls;
mod recovery;
mod resources;
mod typing;
mod updates;
use super::Desktop;
use talk_core::preferences::Locale;

pub fn text(locale: Locale, key: &str) -> &str {
    if locale == Locale::German {
        return key;
    }
    for catalog in [
        call::MESSAGES,
        chat::MESSAGES,
        connection::MESSAGES,
        content::MESSAGES,
        files::MESSAGES,
        polls::MESSAGES,
        conversation::MESSAGES,
        resources::MESSAGES,
        recovery::MESSAGES,
        updates::MESSAGES,
        onboarding::MESSAGES,
        own_status::MESSAGES,
        people::MESSAGES,
        typing::MESSAGES,
    ] {
        if let Ok(index) = catalog.binary_search_by_key(&key, |row| row.0) {
            return if locale == Locale::Spanish {
                catalog[index].2
            } else {
                catalog[index].1
            };
        }
    }
    key
}
#[derive(Clone, Copy)]
pub enum Count {
    Participants,
    VisibleParticipants,
    Replies,
}
impl Desktop {
    pub fn count(&self, kind: Count, n: u64) -> String {
        let key = match (kind, n == 1) {
            (Count::Participants, true) => "{n} teilnehmende Person",
            (Count::Participants, false) => "{n} Teilnehmende",
            (Count::VisibleParticipants, true) => "{n} teilnehmende Person angezeigt",
            (Count::VisibleParticipants, false) => "{n} Teilnehmende angezeigt",
            (Count::Replies, true) => "{n} Antwort",
            (Count::Replies, false) => "{n} Antworten",
        };
        self.lf(key, &[("n", &n.to_string())])
    }

    pub fn l<'a>(&self, key: &'a str) -> &'a str {
        text(self.locale, key)
    }
    pub fn lf(&self, key: &str, values: &[(&str, &str)]) -> String {
        let mut result = self.l(key).to_owned();
        for (name, value) in values.iter().take(8) {
            result = result.replace(&format!("{{{name}}}"), value);
        }
        result
    }
}
/// Format bounded byte counts without depending on the process locale.
pub(super) fn file_size(locale: Locale, size: Option<u64>) -> String {
    let Some(size) = size else {
        return String::new();
    };
    if size < 1024 {
        return format!("{size} B");
    }
    let (value, unit) = if size < 1024 * 1024 {
        (size as f64 / 1024., "KB")
    } else if size < 1024 * 1024 * 1024 {
        (size as f64 / (1024. * 1024.), "MB")
    } else {
        (size as f64 / (1024. * 1024. * 1024.), "GB")
    };
    let number = format!("{value:.1}");
    format!(
        "{} {unit}",
        if locale == Locale::English {
            number
        } else {
            number.replace('.', ",")
        }
    )
}

/// Format the cached local wall time for display; editing retains its fixed input grammar.
pub(super) fn local_datetime(locale: Locale, value: &str) -> Option<String> {
    let (date, time) = value.split_once(' ')?;
    let mut parts = date.split('-');
    let (year, month, day) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some()
        || year.len() != 4
        || month.len() != 2
        || day.len() != 2
        || !date.chars().all(|c| c.is_ascii_digit() || c == '-')
        || time.len() != 5
        || !time
            .chars()
            .enumerate()
            .all(|(i, c)| if i == 2 { c == ':' } else { c.is_ascii_digit() })
    {
        return None;
    }
    Some(match locale {
        Locale::German => format!("{day}.{month}.{year} {time}"),
        Locale::Spanish => format!("{day}/{month}/{year} {time}"),
        Locale::English => format!("{month}/{day}/{year} {time}"),
    })
}

/// The server sends WebDAV dates in HTTP-date form. Only display dates we can
/// recognize; never present protocol text as a localized date.
pub(super) fn file_date(locale: Locale, raw: &str) -> Option<String> {
    let parts: Vec<_> = raw.split_whitespace().collect();
    let day_index = parts
        .iter()
        .position(|part| part.trim_end_matches(',').parse::<u8>().is_ok())?;
    let day = parts
        .get(day_index)?
        .trim_end_matches(',')
        .parse::<u8>()
        .ok()?;
    let month = match *parts.get(day_index + 1)? {
        "Jan" => 1,
        "Feb" => 2,
        "Mar" => 3,
        "Apr" => 4,
        "May" => 5,
        "Jun" => 6,
        "Jul" => 7,
        "Aug" => 8,
        "Sep" => 9,
        "Oct" => 10,
        "Nov" => 11,
        "Dec" => 12,
        _ => return None,
    };
    let year = parts.get(day_index + 2)?.parse::<u16>().ok()?;
    if !(1..=31).contains(&day) || year < 1000 {
        return None;
    }
    Some(match locale {
        Locale::German => format!("{day:02}.{month:02}.{year}"),
        Locale::Spanish => format!("{day:02}/{month:02}/{year}"),
        Locale::English => format!("{month:02}/{day:02}/{year}"),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    fn placeholders(s: &str) -> Vec<&str> {
        s.split('{')
            .skip(1)
            .filter_map(|s| s.split_once('}').map(|(key, _)| key))
            .collect()
    }
    #[test]
    fn file_metadata_uses_locale_and_rejects_unrecognized_dates() {
        assert_eq!(file_size(Locale::German, Some(485_000)), "473,6 KB");
        assert_eq!(file_size(Locale::English, Some(485_000)), "473.6 KB");
        assert_eq!(file_size(Locale::Spanish, Some(485_000)), "473,6 KB");
        assert_eq!(
            file_date(Locale::German, "Wed, 04 Nov 2026 12:00:00 GMT").as_deref(),
            Some("04.11.2026")
        );
        assert_eq!(
            file_date(Locale::English, "04 Nov 2026").as_deref(),
            Some("11/04/2026")
        );
        assert_eq!(
            file_date(Locale::Spanish, "04 Nov 2026").as_deref(),
            Some("04/11/2026")
        );
        assert!(file_date(Locale::German, "unrecognized").is_none());
        assert_eq!(
            local_datetime(Locale::German, "2026-11-04 09:41").as_deref(),
            Some("04.11.2026 09:41")
        );
        assert_eq!(
            local_datetime(Locale::Spanish, "2026-11-04 09:41").as_deref(),
            Some("04/11/2026 09:41")
        );
        assert_eq!(
            local_datetime(Locale::English, "2026-11-04 09:41").as_deref(),
            Some("11/04/2026 09:41")
        );
        assert!(local_datetime(Locale::English, "bad input").is_none());
    }
    #[test]
    fn resource_keys_are_unique_and_translations_preserve_named_arguments() {
        for catalog in [
            resources::MESSAGES,
            recovery::MESSAGES,
            conversation::MESSAGES,
            call::MESSAGES,
            chat::MESSAGES,
            connection::MESSAGES,
            content::MESSAGES,
            files::MESSAGES,
            polls::MESSAGES,
            updates::MESSAGES,
            onboarding::MESSAGES,
            own_status::MESSAGES,
            people::MESSAGES,
            typing::MESSAGES,
        ] {
            let mut last = "";
            for &(key, en, es) in catalog {
                assert!(key > last && !en.is_empty() && !es.is_empty());
                assert_eq!(placeholders(key), placeholders(en), "English: {key}");
                assert_eq!(placeholders(key), placeholders(es), "Spanish: {key}");
                last = key;
            }
        }
        assert_eq!(text(Locale::English, "Allgemein"), "General");
        assert_eq!(text(Locale::Spanish, "Einstellungen"), "Ajustes");
    }
    #[test]
    fn connection_failure_copy_is_translated_on_every_surface() {
        use talk_core::ui::presentation::ConnectionFailure;
        for failure in [
            ConnectionFailure::Unreachable,
            ConnectionFailure::TimedOut,
            ConnectionFailure::AccessDenied,
            ConnectionFailure::ServerUnavailable,
            ConnectionFailure::RateLimited,
            ConnectionFailure::InvalidResponse,
        ] {
            let (title, detail) = crate::presentation::state::connection_failure_copy(failure);
            for key in [
                title,
                detail,
                "Dein Entwurf bleibt erhalten. Nachrichten werden nicht automatisch gesendet.",
            ] {
                for locale in [Locale::English, Locale::Spanish] {
                    assert_ne!(text(locale, key), key, "missing translation: {key}");
                }
            }
        }
    }
    #[test]
    fn account_loading_failures_and_recovery_actions_are_translated() {
        use talk_core::ui::presentation::AccountLoadFailure;
        for failure in [
            AccountLoadFailure::DeviceChanged,
            AccountLoadFailure::InUse,
            AccountLoadFailure::AccessDenied,
            AccountLoadFailure::Unavailable,
            AccountLoadFailure::UnsafeLocation,
            AccountLoadFailure::Incomplete,
            AccountLoadFailure::BindingMismatch,
            AccountLoadFailure::UnsupportedVersion,
            AccountLoadFailure::InvalidData,
            AccountLoadFailure::RecoveryRequired,
        ] {
            let (title, detail) = crate::presentation::state::account_load_failure_copy(failure);
            for key in [
                title,
                detail,
                "Konten erneut laden",
                "Kontospeicher wieder verwenden",
            ] {
                for locale in [Locale::English, Locale::Spanish] {
                    assert_ne!(text(locale, key), key, "missing translation: {key}");
                }
            }
        }
    }
}
