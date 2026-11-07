//! Bounded NextCloud unified-search metadata and account-wide message query.
//! Provider behavior: Talk 24/25 `lib/Search/MessageSearch.php`; NextCloud 34/35
//! `core/Controller/UnifiedSearchController.php`. Returned URLs are never fetched.
use crate::{ErrorKind, Kind, Plan, bounded, chat, models::RoomToken};
use reqwest::Method;
use serde_json::Value;

#[derive(Clone, Debug, Default)]
pub struct Provider {
    pub term: bool,
    pub conversation: bool,
    pub person: bool,
    pub since: bool,
    pub until: bool,
}
#[derive(Clone, Debug)]
pub struct Query {
    pub term: String,
    pub conversation: Option<RoomToken>,
    pub person: Option<String>,
    pub since: Option<String>,
    pub until: Option<String>,
    pub cursor: Option<String>,
}
pub(crate) fn providers(value: &Value) -> Result<crate::Reply, ErrorKind> {
    let values = value.as_array().ok_or(ErrorKind::Malformed)?;
    if values.len() > 256 {
        return Err(ErrorKind::TooLarge);
    }
    let mut found = None;
    for item in values {
        let id = item
            .get("id")
            .and_then(Value::as_str)
            .ok_or(ErrorKind::Malformed)?;
        bounded(id, 128)?;
        if id != "talk-message" {
            continue;
        }
        if found.is_some() {
            return Err(ErrorKind::Malformed);
        }
        let filters = item
            .get("filters")
            .and_then(Value::as_object)
            .ok_or(ErrorKind::Malformed)?;
        if filters.len() > 64 {
            return Err(ErrorKind::TooLarge);
        }
        found = Some(Provider {
            term: filters.get("term").and_then(Value::as_str) == Some("string"),
            conversation: filters.get("conversation").and_then(Value::as_str) == Some("string"),
            person: filters.get("person").and_then(Value::as_str) == Some("person"),
            since: filters.get("since").and_then(Value::as_str) == Some("datetime"),
            until: filters.get("until").and_then(Value::as_str) == Some("datetime"),
        });
    }
    Ok(crate::Reply::SearchProvider(found))
}
pub(crate) fn plan(query: &Query) -> Result<Plan, ErrorKind> {
    bounded(&query.term, 256)?;
    if query.term.trim().is_empty() {
        return Err(ErrorKind::Malformed);
    }
    let mut params = vec![("term", query.term.clone()), ("limit", "50".into())];
    if let Some(room) = &query.conversation {
        params.push(("conversation", room.as_str().into()));
    }
    for (key, value, max) in [
        ("person", &query.person, 256),
        ("since", &query.since, 32),
        ("until", &query.until, 32),
        ("cursor", &query.cursor, 1024),
    ] {
        if let Some(value) = value {
            bounded(value, max)?;
            if value.is_empty() || value.chars().any(char::is_control) {
                return Err(ErrorKind::Malformed);
            }
            params.push((key, value.clone()));
        }
    }
    Ok((
        Method::GET,
        "ocs/v2.php/search/providers/talk-message/search".into(),
        params,
        vec![],
        Kind::Chat(chat::ResponseKind::Search),
        false,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn discovery_and_query_preserve_supported_filters_and_bound_input() {
        let crate::Reply::SearchProvider(Some(provider)) = providers(&json!([
            {"id":"talk-message","filters":{"term":"string","conversation":"string","person":"person","since":"datetime","until":"datetime"}}
        ])).unwrap() else { panic!("provider") };
        assert!(
            provider.term
                && provider.conversation
                && provider.person
                && provider.since
                && provider.until
        );
        let mut query = Query {
            term: "hello".into(),
            conversation: Some(RoomToken::parse("testroom").unwrap()),
            person: Some("alice".into()),
            since: Some("2026-01-01".into()),
            until: None,
            cursor: Some("50".into()),
        };
        let (_, route, params, _, _, _) = plan(&query).unwrap();
        assert_eq!(route, "ocs/v2.php/search/providers/talk-message/search");
        assert!(params.contains(&("conversation", "testroom".into())));
        assert!(params.contains(&("person", "alice".into())));
        assert!(params.contains(&("cursor", "50".into())));
        query.term = " ".into();
        assert!(matches!(plan(&query), Err(ErrorKind::Malformed)));
        assert!(matches!(
            providers(
                &json!([{"id":"talk-message","filters":{}},{"id":"talk-message","filters":{}}])
            ),
            Err(ErrorKind::Malformed)
        ));
    }
}
