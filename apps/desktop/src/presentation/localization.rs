//! Own interface copy only. User, room and message content never enters this catalog.
//! Source keys are stable German author strings; translations remain centralized.
mod chat;
mod content;
mod conversation;
mod onboarding;
mod resources;
mod updates;
use super::Desktop;
use talk_core::preferences::Locale;

pub fn text(locale: Locale, key: &str) -> &str {
    if locale == Locale::German {
        return key;
    }
    for catalog in [
        chat::MESSAGES,
        content::MESSAGES,
        conversation::MESSAGES,
        resources::MESSAGES,
        updates::MESSAGES,
        onboarding::MESSAGES,
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
    fn resource_keys_are_unique_and_translations_preserve_named_arguments() {
        for catalog in [
            resources::MESSAGES,
            conversation::MESSAGES,
            chat::MESSAGES,
            content::MESSAGES,
            updates::MESSAGES,
            onboarding::MESSAGES,
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
}
