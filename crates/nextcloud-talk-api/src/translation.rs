//! Nextcloud core OCS translation API, pinned to the NC34/35 client contract.
//! https://docs.nextcloud.com/server/35/developer_manual/client_apis/OCS/ocs-translation-api.html
use crate::ErrorKind;
use serde_json::Value;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Languages {
    pub detection: bool,
    pub targets: Vec<String>,
}

pub(crate) fn valid_language(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
}
impl Languages {
    pub(crate) fn decode(value: &Value) -> Result<Self, ErrorKind> {
        let detection = value
            .get("languageDetection")
            .and_then(Value::as_bool)
            .ok_or(ErrorKind::Malformed)?;
        let pairs = value
            .get("languages")
            .and_then(Value::as_array)
            .ok_or(ErrorKind::Malformed)?;
        if pairs.len() > 256 {
            return Err(ErrorKind::TooLarge);
        }
        let mut targets = Vec::new();
        for pair in pairs {
            let from = pair
                .get("from")
                .and_then(Value::as_str)
                .ok_or(ErrorKind::Malformed)?;
            let to = pair
                .get("to")
                .and_then(Value::as_str)
                .ok_or(ErrorKind::Malformed)?;
            if !valid_language(from) || !valid_language(to) {
                return Err(ErrorKind::Malformed);
            }
            if !targets.iter().any(|target| target == to) {
                if targets.len() == 64 {
                    return Err(ErrorKind::TooLarge);
                }
                targets.push(to.to_owned());
            }
        }
        Ok(Self { detection, targets })
    }
}
pub(crate) fn translated(value: &Value) -> Result<String, ErrorKind> {
    let text = value
        .get("text")
        .and_then(Value::as_str)
        .ok_or(ErrorKind::Malformed)?;
    if text.len() > 4096 {
        return Err(ErrorKind::TooLarge);
    }
    Ok(text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn language_options_are_bounded_and_require_well_formed_pairs() {
        let languages = Languages::decode(&json!({"languageDetection":true,"languages":[{"from":"en","to":"de"},{"from":"es","to":"de"}]})).unwrap();
        assert!(languages.detection);
        assert_eq!(languages.targets, ["de"]);
        assert!(
            Languages::decode(
                &json!({"languageDetection":true,"languages":[{"from":"en","to":"de?x"}]})
            )
            .is_err()
        );
        assert!(translated(&json!({"text":"ok"})).is_ok());
        assert!(translated(&json!({"text":"x".repeat(4097)})).is_err());
    }
}
