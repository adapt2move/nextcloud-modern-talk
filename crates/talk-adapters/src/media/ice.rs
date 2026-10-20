//! Capabilities of the pinned async engine, not merely URI syntax. In released
//! webrtc 0.21.1, transport/turn_relayer.rs rejects secure/non-UDP TURN and
//! transport/stun_gatherer.rs rejects secure STUN. Do not pass those alternatives
//! through and let its warnings silently imply supported traversal.
use super::{Error, IceServer};
use rtc::ice::url::{SchemeType, Url};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum IcePolicy {
    #[default]
    All,
    Relay,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnsupportedTransport {
    StunTcp,
    StunTls,
    TurnTcp,
    TurnTls,
}
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct IceSupport {
    pub stun_udp: bool,
    pub turn_udp: bool,
    /// Distinct advertised alternatives that this engine cannot use. No URLs,
    /// usernames or credentials enter this report.
    pub unsupported: Vec<UnsupportedTransport>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(
    dead_code,
    reason = "Selected-pair evidence queried by the native qualification executable"
)]
pub enum CandidateKind {
    Host,
    ServerReflexive,
    PeerReflexive,
    Relay,
    Unknown,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SelectedPair {
    pub local: CandidateKind,
    pub remote: CandidateKind,
    pub udp: bool,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct IceEvidence {
    pub host_candidates: u16,
    pub server_reflexive_candidates: u16,
    pub relay_candidates: u16,
    pub selected: Option<SelectedPair>,
}

pub(super) fn plan(
    servers: Vec<IceServer>,
    policy: IcePolicy,
) -> Result<(Vec<IceServer>, IceSupport), Error> {
    if servers.len() > 8 {
        return Err(Error::Malformed);
    }
    let configured = !servers.is_empty();
    let mut usable = Vec::new();
    let mut support = IceSupport::default();
    for mut server in servers {
        if server.urls.is_empty()
            || server.urls.len() > 4
            || server.username.len() > 1024
            || server.password.len() > 2048
        {
            return Err(Error::Malformed);
        }
        let mut urls = Vec::new();
        for raw in server.urls {
            if raw.len() > 2048 || raw.bytes().any(|b| b.is_ascii_control() || b == b' ') {
                return Err(Error::Malformed);
            }
            // STUN's grammar has no query. Talk deployments sometimes append
            // transport=udp; accept only that exact extension, and report tcp
            // explicitly instead of stripping it into an apparent UDP success.
            let stun = raw.starts_with("stun:") || raw.starts_with("stuns:");
            let (parse, explicit_tcp) = if stun {
                match raw.split_once('?') {
                    Some((base, "transport=udp")) => (base, false),
                    Some((base, "transport=tcp")) => (base, true),
                    Some(_) => return Err(Error::Malformed),
                    None => (raw.as_str(), false),
                }
            } else {
                (raw.as_str(), false)
            };
            let url = Url::parse_url(parse).map_err(|_| Error::Malformed)?;
            let omitted = match url.scheme {
                SchemeType::Stuns => Some(UnsupportedTransport::StunTls),
                SchemeType::Stun if explicit_tcp => Some(UnsupportedTransport::StunTcp),
                SchemeType::Stun => {
                    support.stun_udp = true;
                    None
                }
                SchemeType::Turns => Some(UnsupportedTransport::TurnTls),
                SchemeType::Turn if url.proto.to_string() != "udp" => {
                    Some(UnsupportedTransport::TurnTcp)
                }
                SchemeType::Turn => {
                    if server.username.is_empty() || server.password.is_empty() {
                        return Err(Error::Malformed);
                    }
                    support.turn_udp = true;
                    None
                }
                _ => return Err(Error::Malformed),
            };
            if let Some(omitted) = omitted {
                if !support.unsupported.contains(&omitted) {
                    support.unsupported.push(omitted);
                }
            } else {
                urls.push(parse.to_owned());
            }
        }
        if !urls.is_empty() {
            server.urls = urls;
            usable.push(server);
        }
    }
    if (configured && usable.is_empty()) || (policy == IcePolicy::Relay && !support.turn_udp) {
        return Err(Error::Unsupported);
    }
    Ok((usable, support))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn server(urls: &[&str]) -> IceServer {
        IceServer {
            urls: urls.iter().map(|s| s.to_string()).collect(),
            username: "fixture".into(),
            password: "opaque-fixture".into(),
        }
    }
    #[test]
    fn mixed_settings_keep_udp_and_report_each_unsupported_alternative() {
        let (servers, report) = plan(
            vec![server(&[
                "turn:127.0.0.1:3478?transport=udp",
                "turn:127.0.0.1:3478?transport=tcp",
                "turns:127.0.0.1:5349?transport=tcp",
            ])],
            IcePolicy::Relay,
        )
        .unwrap();
        assert_eq!(servers[0].urls, ["turn:127.0.0.1:3478?transport=udp"]);
        assert!(report.turn_udp);
        assert_eq!(
            report.unsupported,
            [UnsupportedTransport::TurnTcp, UnsupportedTransport::TurnTls]
        );
    }
    #[test]
    fn unsupported_only_and_relay_without_turn_do_not_fall_back_to_host() {
        assert!(matches!(
            plan(vec![server(&["turns:127.0.0.1:5349"])], IcePolicy::All),
            Err(Error::Unsupported)
        ));
        assert!(matches!(
            plan(vec![server(&["stun:127.0.0.1:3478"])], IcePolicy::Relay),
            Err(Error::Unsupported)
        ));
        assert!(matches!(
            plan(Vec::new(), IcePolicy::Relay),
            Err(Error::Unsupported)
        ));
        assert!(plan(Vec::new(), IcePolicy::All).is_ok());
    }
    #[test]
    fn malformed_or_missing_credentials_and_stun_tcp_are_distinct() {
        let mut missing = server(&["turn:127.0.0.1:3478"]);
        missing.password.clear();
        assert!(matches!(
            plan(vec![missing], IcePolicy::All),
            Err(Error::Malformed)
        ));
        assert!(matches!(
            plan(
                vec![server(&["stun:127.0.0.1?transport=magic"])],
                IcePolicy::All
            ),
            Err(Error::Malformed)
        ));
        let (_, report) = plan(
            vec![server(&[
                "stun:127.0.0.1?transport=tcp",
                "stun:127.0.0.1?transport=udp",
            ])],
            IcePolicy::All,
        )
        .unwrap();
        assert!(report.stun_udp);
        assert_eq!(report.unsupported, [UnsupportedTransport::StunTcp]);
    }
}
