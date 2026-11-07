//! NextCloud's authenticated reference resolver. No linked site is fetched by the client.
use crate::{ErrorKind, Kind, Plan, bounded, models::string};
use reqwest::Method;
use serde_json::Value;

pub struct Resolved {
    pub reference: String,
    pub title: Option<String>,
    pub description: Option<String>,
    /// Untrusted until the transport accepts it as an installation-owned preview route.
    pub thumbnail: Option<String>,
}

pub(crate) fn plan(reference: &str) -> Result<Plan, ErrorKind> {
    bounded(reference, 2048)?;
    let url = url::Url::parse(reference).map_err(|_| ErrorKind::Malformed)?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || reference.chars().any(char::is_control)
    {
        return Err(ErrorKind::Malformed);
    }
    Ok((
        Method::GET,
        "ocs/v2.php/references/resolve".into(),
        vec![("reference", reference.into())],
        vec![],
        Kind::Reference,
        false,
    ))
}

pub(crate) fn decode(value: &Value) -> Result<crate::Reply, ErrorKind> {
    let values = value
        .get("references")
        .and_then(Value::as_object)
        .ok_or(ErrorKind::Malformed)?;
    if values.len() > 1 {
        return Err(ErrorKind::TooLarge);
    }
    let Some((reference, result)) = values.iter().next() else {
        return Ok(crate::Reply::Reference(None));
    };
    bounded(reference, 2048)?;
    if result.is_null() {
        return Ok(crate::Reply::Reference(None));
    }
    if result.get("accessible").and_then(Value::as_bool) == Some(false) {
        return Ok(crate::Reply::Reference(None));
    }
    let object = result
        .get("openGraphObject")
        .filter(|object| object.is_object() && object.get("name").is_some())
        .or_else(|| result.get("richObject").filter(|object| object.is_object()));
    let Some(object) = object else {
        return Ok(crate::Reply::Reference(None));
    };
    Ok(crate::Reply::Reference(Some(Resolved {
        reference: reference.clone(),
        title: string(object.get("name"), 512)?,
        description: string(object.get("description"), 2048)?,
        thumbnail: string(object.get("thumb"), 2048)?,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn resolver_requires_safe_web_urls_and_bounded_single_result() {
        assert!(plan("https://example.test/article").is_ok());
        assert!(plan("file:///tmp/secret").is_err());
        assert!(plan("https://user:pass@example.test/").is_err());
        assert!(matches!(
            decode(&json!({"references": {"https://a.test": null,"https://b.test": null}})),
            Err(ErrorKind::TooLarge)
        ));
    }
}
