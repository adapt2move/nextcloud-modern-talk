use crate::ErrorKind;
use serde_json::{Value, json};
use url::Url;
use zeroize::Zeroizing;

/// Server-issued HPB credentials. They are never application credentials and
/// have no public getter, Debug or serialization implementation.
#[derive(Clone)]
pub struct ExternalSettings {
    pub(super) servers: Vec<Url>,
    pub(super) user: Zeroizing<String>,
    v1: Option<Zeroizing<String>>,
    v2: Option<Zeroizing<String>>,
}

impl ExternalSettings {
    pub(crate) fn decode(value: &Value) -> Result<Self, ErrorKind> {
        // Federation has a different backend/authentication route. Never infer
        // that route from an ordinary local account's authentication.
        if value["federation"]
            .as_object()
            .is_some_and(|v| !v.is_empty())
        {
            return Err(ErrorKind::Unsupported);
        }
        let user = super::wire::text(&value["userId"], 256)?;
        let raw = match &value["server"] {
            Value::String(s) => vec![s.as_str()],
            Value::Array(v) if !v.is_empty() && v.len() <= 4 => v
                .iter()
                .map(|v| super::wire::text(v, 2048))
                .collect::<Result<Vec<_>, _>>()?,
            _ => return Err(ErrorKind::Malformed),
        };
        let servers = raw.into_iter().map(endpoint).collect::<Result<_, _>>()?;
        let params = &value["helloAuthParams"];
        if params.get("internal").is_some() {
            return Err(ErrorKind::Unsupported);
        }
        let encode = |v: &Value| -> Result<Zeroizing<String>, ErrorKind> {
            let map = v.as_object().ok_or(ErrorKind::Malformed)?;
            if map.is_empty() || map.len() > 16 {
                return Err(ErrorKind::TooLarge);
            }
            let encoded = serde_json::to_string(v).map_err(|_| ErrorKind::Malformed)?;
            if encoded.len() > 16 * 1024 {
                return Err(ErrorKind::TooLarge);
            }
            Ok(Zeroizing::new(encoded))
        };
        let v1 = if let Some(v) = params.get("1.0") {
            if v["userid"].as_str() != Some(user) {
                return Err(ErrorKind::WrongScope);
            }
            super::wire::text(&v["ticket"], 8192)?;
            Some(encode(v)?)
        } else if let Some(ticket) = value.get("ticket") {
            let ticket = super::wire::text(ticket, 8192)?;
            Some(encode(&json!({"userid":user,"ticket":ticket}))?)
        } else {
            None
        };
        let v2 = if let Some(v) = params.get("2.0") {
            super::wire::text(&v["token"], 16 * 1024)?;
            Some(encode(v)?)
        } else {
            None
        };
        if v1.is_none() && v2.is_none() {
            return Err(ErrorKind::Malformed);
        }
        Ok(Self {
            servers,
            user: Zeroizing::new(user.to_owned()),
            v1,
            v2,
        })
    }

    pub(super) fn hello(
        &self,
        id: &str,
        backend: &str,
        hello_v2: bool,
    ) -> Result<Value, ErrorKind> {
        let (version, params) = if hello_v2 && self.v2.is_some() {
            ("2.0", self.v2.as_ref())
        } else {
            ("1.0", self.v1.as_ref())
        };
        let params = params.ok_or(ErrorKind::Unsupported)?;
        let params = crate::json::json(params.as_bytes())?;
        Ok(json!({"id":id,"type":"hello","hello":{
            "version":version,"auth":{"url":backend,"params":params}
        }}))
    }
}

fn endpoint(input: &str) -> Result<Url, ErrorKind> {
    if input.len() > 2048
        || input.trim() != input
        || input.contains(['\\', '?', '#', '@', '%'])
        || input.chars().any(char::is_control)
    {
        return Err(ErrorKind::Malformed);
    }
    let path = input
        .split_once("://")
        .ok_or(ErrorKind::Malformed)?
        .1
        .split_once('/')
        .map_or("", |(_, p)| p);
    if path.contains("//") || path.split('/').any(|p| p == "." || p == "..") {
        return Err(ErrorKind::Malformed);
    }
    let mut url = Url::parse(input).map_err(|_| ErrorKind::Malformed)?;
    if url.host_str().is_none_or(|h| h.ends_with('.'))
        || url.port() == Some(0)
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(ErrorKind::Malformed);
    }
    let secure = matches!(url.scheme(), "https" | "wss");
    let loopback = cfg!(any(test, feature = "backend-test"))
        && matches!(url.scheme(), "http" | "ws")
        && matches!(url.host_str(), Some("127.0.0.1" | "[::1]"));
    if !secure && !loopback {
        return Err(ErrorKind::Unsupported);
    }
    url.set_scheme(if secure { "wss" } else { "ws" })
        .map_err(|_| ErrorKind::Malformed)?;
    url.set_path(&format!("{}/spreed", url.path().trim_end_matches('/')));
    Ok(url)
}
