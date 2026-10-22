//! Login Flow v2: no cookies, redirects, retries, trust overrides or primary password.
use super::error::{IdentityError, Result};
use crate::nextcloud::ServerOrigin;
use reqwest::{Client, StatusCode};
use serde::de::{self, DeserializeSeed, MapAccess, Visitor};
use std::time::{Duration, Instant};
use talk_core::{lifecycle::Cancellation, ports::ErrorKind};
use url::Url;
use zeroize::Zeroizing;

const BODY_LIMIT: usize = 16 * 1024;
const REQUEST_TIME: Duration = Duration::from_secs(3);
const FLOW_TIME: Duration = Duration::from_secs(20 * 60);
const POLL_INTERVAL: Duration = Duration::from_secs(2);
pub(super) struct Origin {
    pub text: String,
    pub fixture: bool,
}
impl Origin {
    pub fn parse(input: &str, fixture: bool) -> Result<Self> {
        let input = if input.contains("://") {
            input.to_owned()
        } else {
            format!("https://{input}")
        };
        if fixture {
            #[cfg(any(test, feature = "backend-test"))]
            {
                ServerOrigin::loopback(&input)?;
            }
            #[cfg(not(any(test, feature = "backend-test")))]
            {
                return Err(ErrorKind::Unsupported.into());
            }
        } else {
            ServerOrigin::https(&input)?;
        }
        let mut url = Url::parse(&input).map_err(|_| ErrorKind::Malformed)?;
        if !url.path().ends_with('/') {
            url.set_path(&format!("{}/", url.path()));
        }
        Ok(Self {
            text: url.to_string(),
            fixture,
        })
    }
    pub fn transport(&self) -> Result<ServerOrigin> {
        if self.fixture {
            #[cfg(any(test, feature = "backend-test"))]
            {
                return ServerOrigin::loopback(&self.text).map_err(Into::into);
            }
        }
        ServerOrigin::https(&self.text).map_err(Into::into)
    }
    pub(super) fn display_origin(&self) -> Result<talk_core::bounded::Text<512>> {
        let url = Url::parse(&self.text).map_err(|_| ErrorKind::Malformed)?;
        talk_core::bounded::Text::new(&url.origin().ascii_serialization())
            .map_err(|_| ErrorKind::TooLarge.into())
    }
    /// Non-authoritative chooser label: no scheme, base path, user or grant route.
    pub fn label(&self) -> Option<talk_core::bounded::Text<128>> {
        let url = Url::parse(&self.text).ok()?;
        let host = url.host()?;
        let label = match url.port() {
            Some(port) => format!("{host}:{port}"),
            None => host.to_string(),
        };
        talk_core::bounded::Text::new(&label).ok()
    }
    fn endpoint(&self, suffix: &str) -> String {
        format!("{}{suffix}", self.text)
    }
    fn poll_url(&self, input: &str) -> Result<()> {
        if !["index.php/login/v2/poll", "login/v2/poll"]
            .iter()
            .any(|route| input == self.endpoint(route))
        {
            return Err(ErrorKind::WrongScope.into());
        }
        Ok(())
    }
    fn browser_url(&self, input: &str) -> Result<()> {
        if input.len() > 2048 {
            return Err(ErrorKind::TooLarge.into());
        }
        let prefixes = [
            self.endpoint("index.php/login/v2/flow/"),
            self.endpoint("login/v2/flow/"),
        ];
        let token = prefixes
            .iter()
            .find_map(|prefix| input.strip_prefix(prefix))
            .ok_or(ErrorKind::WrongScope)?;
        if token.is_empty()
            || token.len() > 512
            || !token
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        {
            return Err(ErrorKind::Malformed.into());
        }
        Ok(())
    }
}

#[cfg(test)]
#[test]
fn selected_origin_label_normalizes_host_and_never_exposes_a_login_route() {
    for (input, expected) in [
        ("Cloud.Example.test/nextcloud", "https://cloud.example.test"),
        (
            "https://Cloud.Example.test:443/nextcloud/",
            "https://cloud.example.test",
        ),
        (
            "https://cloud.example.test:8443/index.php/login/v2/flow/private",
            "https://cloud.example.test:8443",
        ),
    ] {
        assert_eq!(
            super::selected_server_origin(input).unwrap().as_str(),
            expected
        );
    }
    for input in [
        "https://user:password@cloud.example.test",
        "https://cloud.example.test/?token=private",
        "https://cloud.example.test/#private",
        "http://cloud.example.test",
    ] {
        assert!(super::selected_server_origin(input).is_err());
    }
}

pub(super) struct Grant {
    pub origin: Origin,
    pub user: Zeroizing<String>,
    pub login_name: Zeroizing<String>,
    pub password: Zeroizing<String>,
}
pub(super) struct LoginFlow {
    client: Client,
    origin: Origin,
    token: Zeroizing<String>,
    endpoint: Zeroizing<String>,
    browser: Zeroizing<String>,
    expires: Instant,
    next_poll: Instant,
    cancellation: Cancellation,
}
pub(super) enum Poll {
    Pending,
    Granted(Grant),
}
impl LoginFlow {
    pub async fn start(origin: Origin, cancellation: Cancellation) -> Result<Self> {
        let started = Instant::now();
        let client = Client::builder()
            .https_only(!origin.fixture)
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .no_proxy()
            .referer(false)
            .http1_only()
            .http1_max_headers(32)
            .no_gzip()
            .no_brotli()
            .no_deflate()
            .no_zstd()
            .connect_timeout(REQUEST_TIME)
            .timeout(REQUEST_TIME)
            .pool_max_idle_per_host(1)
            .user_agent("Nextcloud Modern Talk")
            .build()
            .map_err(|_| ErrorKind::Transport)?;
        let response = request(
            &client,
            &origin.endpoint("index.php/login/v2"),
            None,
            &cancellation,
        )
        .await?;
        if response.0 != StatusCode::OK {
            return Err(status(response.0));
        }
        let mut json = parse(&response.1)?;
        let browser = json.take_string("login")?;
        let mut poll = json.take_map("poll")?;
        let token = poll.take_string("token")?;
        let endpoint = poll.take_string("endpoint")?;
        if !json.empty()
            || !poll.empty()
            || token.is_empty()
            || token.len() > 512
            || token.chars().any(char::is_control)
        {
            return Err(ErrorKind::Malformed.into());
        }
        origin.poll_url(&endpoint)?;
        origin.browser_url(&browser)?;
        let now = Instant::now();
        Ok(Self {
            client,
            origin,
            token,
            endpoint,
            browser,
            expires: started + FLOW_TIME,
            next_poll: now,
            cancellation,
        })
    }
    /// Used only by the private OS browser adapter; never returned to core/UI/logs.
    pub fn browser(&self) -> &str {
        &self.browser
    }
    pub fn expires(&self) -> Instant {
        self.expires
    }
    pub async fn poll(&mut self) -> Result<Poll> {
        if self.cancellation.is_cancelled() {
            return Err(ErrorKind::Cancelled.into());
        }
        let now = Instant::now();
        if now >= self.expires {
            return Err(ErrorKind::Deadline.into());
        }
        if now < self.next_poll {
            return Ok(Poll::Pending);
        }
        self.next_poll = now + POLL_INTERVAL;
        let (code, body) = request(
            &self.client,
            &self.endpoint,
            Some(&self.token),
            &self.cancellation,
        )
        .await?;
        if code == StatusCode::NOT_FOUND {
            return Ok(Poll::Pending);
        }
        // A lost one-time grant response is terminal; caller must start a fresh flow.
        if code != StatusCode::OK {
            return Err(status(code));
        }
        let mut json = parse(&body)?;
        let server = json.take_string("server")?;
        let user = json.take_string("loginName")?;
        let password = json.take_string("appPassword")?;
        if !json.empty()
            || user.is_empty()
            || user.len() > 256
            || user.contains(':')
            || user.chars().any(char::is_control)
            || password.is_empty()
            || password.len() > 8192
            || password.chars().any(char::is_control)
        {
            return Err(ErrorKind::Malformed.into());
        }
        let canonical = Origin::parse(&server, self.origin.fixture)?;
        if canonical.text != self.origin.text {
            return Err(ErrorKind::WrongScope.into());
        }
        if self.cancellation.is_cancelled() || Instant::now() >= self.expires {
            return Err(ErrorKind::Cancelled.into());
        }
        Ok(Poll::Granted(Grant {
            origin: canonical,
            user: Zeroizing::new(String::new()),
            login_name: user,
            password,
        }))
    }
}
fn status(code: StatusCode) -> IdentityError {
    if code == StatusCode::FORBIDDEN {
        ErrorKind::ServerAccessDenied
    } else if code == StatusCode::UNAUTHORIZED {
        ErrorKind::ReauthenticationRequired
    } else if code.is_redirection() {
        ErrorKind::RedirectRejected
    } else if code == StatusCode::TOO_MANY_REQUESTS {
        ErrorKind::RateLimited
    } else if code.is_server_error() {
        ErrorKind::ServerUnavailable
    } else {
        ErrorKind::Malformed
    }
    .into()
}
async fn request(
    client: &Client,
    url: &str,
    token: Option<&str>,
    cancel: &Cancellation,
) -> Result<(StatusCode, Zeroizing<Vec<u8>>)> {
    let task = async {
        let mut req = client
            .post(url)
            .header("Accept", "application/json")
            .header("Accept-Encoding", "identity")
            .header("Cache-Control", "no-store");
        if let Some(token) = token {
            req = req.form(&[("token", token)]);
        }
        let mut response = req.send().await.map_err(|_| ErrorKind::Transport)?;
        let code = response.status();
        if code.is_redirection() {
            return Err(ErrorKind::RedirectRejected.into());
        }
        if response
            .headers()
            .get_all("Content-Encoding")
            .iter()
            .any(|h| h.as_bytes() != b"identity")
            || response
                .content_length()
                .is_some_and(|n| n > BODY_LIMIT as u64)
        {
            return Err(ErrorKind::TooLarge.into());
        }
        if response
            .headers()
            .get_all("Content-Encoding")
            .iter()
            .count()
            > 1
        {
            return Err(ErrorKind::Malformed.into());
        }
        if code == StatusCode::OK {
            let mut types = response.headers().get_all("Content-Type").iter();
            let valid = types.next().and_then(|h| h.to_str().ok()).is_some_and(|h| {
                h.split(';')
                    .next()
                    .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("application/json"))
            });
            if !valid || types.next().is_some() {
                return Err(ErrorKind::Malformed.into());
            }
        }
        let mut bytes = Zeroizing::new(Vec::new());
        while let Some(chunk) = response.chunk().await.map_err(|_| ErrorKind::Transport)? {
            if bytes.len() + chunk.len() > BODY_LIMIT {
                return Err(ErrorKind::TooLarge.into());
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok((code, bytes))
    };
    tokio::pin!(task);
    let deadline = tokio::time::sleep(REQUEST_TIME);
    tokio::pin!(deadline);
    loop {
        if cancel.is_cancelled() {
            return Err(ErrorKind::Cancelled.into());
        }
        tokio::select! {
            result=&mut task=>{if cancel.is_cancelled(){return Err(ErrorKind::Cancelled.into());}return result;},
            _=&mut deadline=>return Err(ErrorKind::Deadline.into()),
            _=tokio::time::sleep(Duration::from_millis(25))=>{},
        }
    }
}
// Only small string/object Login Flow records are accepted; no generic wire Value.
// Every stored string is zeroizing; parsing errors are never formatted.
enum Json {
    String(Zeroizing<String>),
    Object(Vec<(Zeroizing<String>, Json)>),
}
impl Json {
    fn take(&mut self, key: &str) -> Result<Json> {
        let Self::Object(v) = self else {
            return Err(ErrorKind::Malformed.into());
        };
        let index = v
            .iter()
            .position(|(k, _)| k.as_str() == key)
            .ok_or(ErrorKind::Malformed)?;
        Ok(v.swap_remove(index).1)
    }
    fn take_string(&mut self, key: &str) -> Result<Zeroizing<String>> {
        match self.take(key)? {
            Self::String(s) => Ok(s),
            _ => Err(ErrorKind::Malformed.into()),
        }
    }
    fn take_map(&mut self, key: &str) -> Result<Json> {
        match self.take(key)? {
            v @ Self::Object(_) => Ok(v),
            _ => Err(ErrorKind::Malformed.into()),
        }
    }
    fn empty(&self) -> bool {
        matches!(self,Self::Object(v) if v.is_empty())
    }
}
struct Seed {
    depth: u8,
}
impl<'de> DeserializeSeed<'de> for Seed {
    type Value = Json;
    fn deserialize<D: de::Deserializer<'de>>(self, d: D) -> std::result::Result<Json, D::Error> {
        if self.depth > 3 {
            return Err(de::Error::custom("depth"));
        }
        d.deserialize_any(self)
    }
}
impl<'de> Visitor<'de> for Seed {
    type Value = Json;
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("bounded Login Flow object")
    }
    fn visit_str<E: de::Error>(self, s: &str) -> std::result::Result<Json, E> {
        if s.len() > 8192 {
            Err(E::custom("field"))
        } else {
            Ok(Json::String(Zeroizing::new(s.into())))
        }
    }
    fn visit_string<E: de::Error>(self, s: String) -> std::result::Result<Json, E> {
        let s = Zeroizing::new(s);
        if s.len() > 8192 {
            Err(E::custom("field"))
        } else {
            Ok(Json::String(s))
        }
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> std::result::Result<Json, A::Error> {
        let mut fields = Vec::new();
        while let Some(key) = map.next_key::<String>()? {
            let key = Zeroizing::new(key);
            if fields.len() >= 8
                || key.len() > 32
                || fields
                    .iter()
                    .any(|(k, _): &(Zeroizing<String>, Json)| k == &key)
            {
                return Err(de::Error::custom("fields"));
            }
            let value = map.next_value_seed(Seed {
                depth: self.depth + 1,
            })?;
            fields.push((key, value));
        }
        Ok(Json::Object(fields))
    }
}
fn parse(bytes: &[u8]) -> Result<Json> {
    if bytes.len() > BODY_LIMIT {
        return Err(ErrorKind::TooLarge.into());
    }
    let mut d = serde_json::Deserializer::from_slice(bytes);
    let v = Seed { depth: 0 }
        .deserialize(&mut d)
        .map_err(|_| ErrorKind::Malformed)?;
    d.end().map_err(|_| ErrorKind::Malformed)?;
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn refused_login_is_denied_before_parsing_a_non_json_error_body() {
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::TcpListener,
        };

        for (code, expected) in [
            (403, ErrorKind::ServerAccessDenied),
            (401, ErrorKind::ReauthenticationRequired),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let origin = format!("http://{}/", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                tokio::time::timeout(Duration::from_secs(3), async move {
                    let (mut stream, _) = listener.accept().await.unwrap();
                    let mut request = Vec::new();
                    while !request.ends_with(b"\r\n\r\n") {
                        assert!(request.len() < 8192, "bounded fixture request");
                        request.push(stream.read_u8().await.unwrap());
                    }
                    assert!(request.starts_with(b"POST /index.php/login/v2 HTTP/1.1\r\n"));
                    stream
                        .write_all(
                            format!(
                                "HTTP/1.1 {code} Refused\r\nContent-Length: 9\r\nConnection: close\r\n\r\nForbidden"
                            )
                            .as_bytes(),
                        )
                        .await
                        .unwrap();
                })
                .await
                .unwrap();
            });
            let result = LoginFlow::start(
                Origin::parse(&origin, true).unwrap(),
                Cancellation::default(),
            )
            .await;
            server.await.unwrap();
            assert!(matches!(result, Err(IdentityError::Request(kind)) if kind == expected));
        }
    }
    #[test]
    fn flow_routes_are_exact_origin_and_installation_bound() {
        let origin = Origin::parse("https://cloud.example.invalid/install", false).unwrap();
        for base in ["login/v2/", "index.php/login/v2/"] {
            assert!(
                origin
                    .poll_url(&format!("{}{base}poll", origin.text))
                    .is_ok()
            );
            assert!(
                origin
                    .browser_url(&format!("{}{base}flow/Ab_C-7", origin.text))
                    .is_ok()
            );
        }
        for value in [
            "http://cloud.example.invalid/install/login/v2/poll",
            "https://cloud.example.invalid/login/v2/poll",
            "https://cloud.example.invalid.evil/install/login/v2/poll",
            "https://cloud.example.invalid/install/login/v2/poll?x=1",
            "https://cloud.example.invalid/install/../login/v2/poll",
            "https://cloud.example.invalid/install/%6cogin/v2/poll",
        ] {
            assert!(origin.poll_url(value).is_err());
        }
        for suffix in ["../bad", "x?query", "x#fragment", "x%2fbad", "", "x/y"] {
            assert!(
                origin
                    .browser_url(&format!("{}login/v2/flow/{suffix}", origin.text))
                    .is_err()
            );
        }
        assert!(Origin::parse("http://127.0.0.1:8894", false).is_err());
        assert!(Origin::parse("http://localhost:8894", true).is_err());
    }
    #[test]
    fn login_json_rejects_depth_fields_duplicates_types_and_oversize() {
        for value in [
            r#"{"a":"x","a":"y"}"#,
            r#"{"a":{"b":{"c":{"d":"x"}}}}"#,
            r#"{"a":[]}"#,
            r#"{"a":true}"#,
            r#"{} {}"#,
            r#"{"a":"1","b":"2","c":"3","d":"4","e":"5","f":"6","g":"7","h":"8","i":"9"}"#,
        ] {
            assert!(parse(value.as_bytes()).is_err());
        }
        assert!(parse(format!("{{\"a\":\"{}\"}}", "x".repeat(8193)).as_bytes()).is_err());
        assert!(parse(&vec![b' '; BODY_LIMIT + 1]).is_err());
    }
    #[tokio::test]
    async fn poll_rate_expiry_and_cancellation_do_not_dispatch_again() {
        let server = crate::identity::test_server::Server::start(35).await;
        let cancellation = Cancellation::default();
        let mut flow = LoginFlow::start(
            Origin::parse(&server.origin, true).unwrap(),
            cancellation.clone(),
        )
        .await
        .unwrap();
        assert!(matches!(flow.poll().await, Ok(Poll::Pending)));
        assert!(matches!(flow.poll().await, Ok(Poll::Pending)));
        assert_eq!(server.polls.load(std::sync::atomic::Ordering::Acquire), 1);
        flow.expires = Instant::now();
        assert!(matches!(
            flow.poll().await,
            Err(IdentityError::Request(ErrorKind::Deadline))
        ));
        cancellation.cancel();
        assert!(matches!(
            flow.poll().await,
            Err(IdentityError::Request(ErrorKind::Cancelled))
        ));
        assert_eq!(server.polls.load(std::sync::atomic::Ordering::Acquire), 1);
    }
}
