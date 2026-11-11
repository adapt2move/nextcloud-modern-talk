//! TURN over TCP and TLS for an engine that only speaks UDP TURN.
//!
//! The pinned engine (webrtc 0.21.1) allocates relays through UDP sockets and
//! rejects `turn:…?transport=tcp` and `turns:`. Many deployments only offer the
//! TLS route (typically port 443), because it passes firewalls that drop UDP.
//! A [`Bridge`] stands in for such a server: a loopback UDP socket the engine
//! talks to as if it were the relay, whose datagrams are carried over one TCP or
//! TLS connection to the real server in RFC 5766 stream framing. The engine's
//! credentials, nonces and relay allocation pass through untouched; only the
//! hop to the server changes.
//!
//! The server certificate is verified by the operating system's trust store (so
//! a company CA installed in the system is honoured), never by a bundled list.
use super::Error;
use rustls::pki_types::ServerName;
use std::{
    net::SocketAddr,
    sync::{Arc, OnceLock},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UdpSocket,
    sync::mpsc,
    task::JoinHandle,
};
use tokio_rustls::TlsConnector;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Largest TURN message: a 16 bit length plus its header.
const MAX_FRAME: usize = 65_535 + 20;
/// Datagrams from the engine are queued by the kernel; this bounds our own reads.
const READ_CHUNK: usize = 16 * 1024;

/// Where the real TURN server listens.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Target {
    pub host: String,
    pub port: u16,
    pub tls: bool,
}

/// How many stream bytes the first frame at the start of `buffer` spans, and
/// how many of them are payload (ChannelData is padded to four bytes on a
/// stream, but not as a datagram).
#[derive(Debug, Eq, PartialEq)]
pub(super) enum Frame {
    Incomplete,
    Complete { datagram: usize, consumed: usize },
    Invalid,
}
pub(super) fn frame(buffer: &[u8]) -> Frame {
    if buffer.len() < 4 {
        return Frame::Incomplete;
    }
    let length = usize::from(u16::from_be_bytes([buffer[2], buffer[3]]));
    match buffer[0] >> 6 {
        // STUN: 20 byte header, attributes padded to four bytes.
        0b00 => {
            if length % 4 != 0 {
                return Frame::Invalid;
            }
            let total = 20 + length;
            if buffer.len() < total {
                Frame::Incomplete
            } else {
                Frame::Complete {
                    datagram: total,
                    consumed: total,
                }
            }
        }
        // ChannelData: 4 byte header, payload padded on a stream.
        0b01 => {
            let total = 4 + length;
            let consumed = total.next_multiple_of(4);
            if buffer.len() < consumed {
                Frame::Incomplete
            } else {
                Frame::Complete {
                    datagram: total,
                    consumed,
                }
            }
        }
        _ => Frame::Invalid,
    }
}
/// A datagram as stream bytes: ChannelData gains its padding.
pub(super) fn to_stream(datagram: &[u8]) -> Option<Vec<u8>> {
    if datagram.len() < 4 || datagram.len() > MAX_FRAME {
        return None;
    }
    let mut bytes = datagram.to_vec();
    match datagram[0] >> 6 {
        0b00 => {}
        0b01 => bytes.resize(datagram.len().next_multiple_of(4), 0),
        _ => return None,
    }
    // A frame the server will parse as exactly this datagram.
    matches!(frame(&bytes), Frame::Complete { consumed, .. } if consumed == bytes.len())
        .then_some(bytes)
}

/// The system trust store's verdict on the relay's certificate.
pub(super) fn system_tls() -> Result<Arc<rustls::ClientConfig>, Error> {
    static CONFIG: OnceLock<Option<Arc<rustls::ClientConfig>>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            use rustls_platform_verifier::BuilderVerifierExt;
            let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
            let config = rustls::ClientConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .ok()?
                .with_platform_verifier()
                .ok()?
                .with_no_client_auth();
            Some(Arc::new(config))
        })
        .clone()
        .ok_or(Error::Engine)
}

async fn connect(
    target: &Target,
    tls: Option<&Arc<rustls::ClientConfig>>,
) -> Result<Box<dyn Stream>, Error> {
    let tcp = tokio::time::timeout(
        CONNECT_TIMEOUT,
        nextcloud_talk_api::hpb::dial(&target.host, target.port),
    )
    .await
    .map_err(|_| Error::Engine)?
    .map_err(|_| Error::Engine)?;
    if !target.tls {
        return Ok(Box::new(tcp));
    }
    let name = ServerName::try_from(target.host.clone()).map_err(|_| Error::Malformed)?;
    let connector = TlsConnector::from(tls.ok_or(Error::Engine)?.clone());
    let stream = tokio::time::timeout(CONNECT_TIMEOUT, connector.connect(name, tcp))
        .await
        .map_err(|_| Error::Engine)?
        .map_err(|_| Error::Engine)?;
    Ok(Box::new(stream))
}
trait Stream: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + Unpin {}
impl<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + Unpin> Stream for T {}

/// A loopback UDP face for one TCP/TLS TURN server. Dropping it closes both.
pub(super) struct Bridge {
    address: SocketAddr,
    tasks: [JoinHandle<()>; 1],
}
impl Bridge {
    /// The `host:port` to give the engine instead of the real server.
    pub(super) fn address(&self) -> SocketAddr {
        self.address
    }

    pub(super) async fn start(
        target: Target,
        tls: Option<Arc<rustls::ClientConfig>>,
    ) -> Result<Self, Error> {
        let udp = Arc::new(
            UdpSocket::bind("127.0.0.1:0")
                .await
                .map_err(|_| Error::Engine)?,
        );
        let address = udp.local_addr().map_err(|_| Error::Engine)?;
        let task = tokio::spawn(async move {
            let _ = relay(udp, target, tls).await;
        });
        Ok(Self {
            address,
            tasks: [task],
        })
    }
}
impl Drop for Bridge {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

/// The engine allocates a relay per local interface, each from its own UDP
/// source address. Each source is a TURN client of its own, so each gets its own
/// connection to the server, exactly as it would over UDP.
const MAX_CLIENTS: usize = 16;
const QUEUE: usize = 64;

/// Carry datagrams for every source the engine uses until the bridge is dropped.
async fn relay(
    udp: Arc<UdpSocket>,
    target: Target,
    tls: Option<Arc<rustls::ClientConfig>>,
) -> Result<(), Error> {
    use std::collections::HashMap;
    let mut sessions: HashMap<SocketAddr, mpsc::Sender<Vec<u8>>> = HashMap::new();
    let mut datagram = vec![0_u8; MAX_FRAME + 1];
    loop {
        let (n, from) = udp
            .recv_from(&mut datagram)
            .await
            .map_err(|_| Error::Engine)?;
        let Some(bytes) = to_stream(&datagram[..n]) else {
            continue;
        };
        sessions.retain(|_, queue| !queue.is_closed());
        if !sessions.contains_key(&from) {
            if sessions.len() >= MAX_CLIENTS {
                continue;
            }
            let (queue, receive) = mpsc::channel(QUEUE);
            tokio::spawn(session(
                udp.clone(),
                from,
                target.clone(),
                tls.clone(),
                receive,
            ));
            sessions.insert(from, queue);
        }
        if let Some(queue) = sessions.get(&from) {
            // A full queue drops the datagram, as a congested UDP path would.
            let _ = queue.try_send(bytes);
        }
    }
}

/// One TURN client's connection: its datagrams go out, the server's come back to
/// its source address. It ends with the connection or when the bridge is dropped.
async fn session(
    udp: Arc<UdpSocket>,
    client: SocketAddr,
    target: Target,
    tls: Option<Arc<rustls::ClientConfig>>,
    mut outgoing: mpsc::Receiver<Vec<u8>>,
) {
    let Ok(stream) = connect(&target, tls.as_ref()).await else {
        return;
    };
    let (mut read, mut write) = tokio::io::split(stream);
    let reader = tokio::spawn(async move {
        let mut buffer: Vec<u8> = Vec::with_capacity(READ_CHUNK);
        let mut chunk = vec![0_u8; READ_CHUNK];
        loop {
            match read.read(&mut chunk).await {
                Ok(0) | Err(_) => return,
                Ok(n) => buffer.extend_from_slice(&chunk[..n]),
            }
            loop {
                match frame(&buffer) {
                    Frame::Incomplete => break,
                    Frame::Invalid => return,
                    Frame::Complete { datagram, consumed } => {
                        let _ = udp.send_to(&buffer[..datagram], client).await;
                        buffer.drain(..consumed);
                    }
                }
            }
            if buffer.len() > MAX_FRAME * 2 {
                return;
            }
        }
    });
    while let Some(bytes) = outgoing.recv().await {
        if write.write_all(&bytes).await.is_err() {
            break;
        }
    }
    reader.abort();
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
    use tokio::net::TcpListener;

    fn stun(body: &[u8]) -> Vec<u8> {
        assert_eq!(body.len() % 4, 0);
        let mut message = vec![0x00, 0x01];
        message.extend_from_slice(&(body.len() as u16).to_be_bytes());
        message.extend_from_slice(&[0x21, 0x12, 0xa4, 0x42]);
        message.extend_from_slice(&[7; 12]);
        message.extend_from_slice(body);
        message
    }
    fn channel_data(payload: &[u8]) -> Vec<u8> {
        let mut message = vec![0x40, 0x01];
        message.extend_from_slice(&(payload.len() as u16).to_be_bytes());
        message.extend_from_slice(payload);
        message
    }

    #[test]
    fn stream_framing_matches_rfc_5766_for_stun_and_channel_data() {
        let request = stun(&[1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(frame(&request[..3]), Frame::Incomplete);
        assert_eq!(frame(&request[..27]), Frame::Incomplete);
        assert_eq!(
            frame(&request),
            Frame::Complete {
                datagram: 28,
                consumed: 28
            }
        );
        // A second message behind the first is left for the next call.
        let mut two = request.clone();
        two.extend_from_slice(&request);
        assert_eq!(
            frame(&two),
            Frame::Complete {
                datagram: 28,
                consumed: 28
            }
        );
        // ChannelData of 5 payload bytes is 9 bytes as a datagram, 12 on a stream.
        let data = channel_data(&[9; 5]);
        assert_eq!(data.len(), 9);
        let streamed = to_stream(&data).unwrap();
        assert_eq!(streamed.len(), 12);
        assert_eq!(
            frame(&streamed),
            Frame::Complete {
                datagram: 9,
                consumed: 12
            }
        );
        assert_eq!(frame(&streamed[..11]), Frame::Incomplete);
        // Anything else is not TURN and ends the connection instead of guessing.
        assert_eq!(frame(&[0xc0, 0, 0, 4, 0, 0, 0, 0]), Frame::Invalid);
        assert_eq!(frame(&[0x00, 0x01, 0x00, 0x03, 0, 0, 0]), Frame::Invalid);
        assert!(to_stream(&[0xc0, 0, 0, 0]).is_none());
        assert!(to_stream(&[0x00, 1]).is_none());
        assert_eq!(to_stream(&request).unwrap(), request);
    }

    /// A stand-in TURN server: echoes each frame back, upper-casing ChannelData.
    async fn serve<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(mut stream: S) {
        let mut buffer = Vec::new();
        let mut chunk = [0_u8; 4096];
        loop {
            let n = match stream.read(&mut chunk).await {
                Ok(0) | Err(_) => return,
                Ok(n) => n,
            };
            buffer.extend_from_slice(&chunk[..n]);
            while let Frame::Complete { datagram, consumed } = frame(&buffer) {
                let mut reply = buffer[..datagram].to_vec();
                if reply[0] >> 6 == 0b00 {
                    reply[0] = 0x01; // success response class
                } else {
                    reply[4..].make_ascii_uppercase();
                }
                reply.resize(reply.len().next_multiple_of(4), 0);
                if stream.write_all(&reply).await.is_err() {
                    return;
                }
                buffer.drain(..consumed);
            }
        }
    }

    async fn recv(engine: &UdpSocket) -> (Vec<u8>, SocketAddr) {
        let mut buffer = vec![0_u8; 2048];
        let (n, from) = tokio::time::timeout(Duration::from_secs(3), engine.recv_from(&mut buffer))
            .await
            .expect("a reply arrives")
            .unwrap();
        buffer.truncate(n);
        (buffer, from)
    }

    async fn exchange(bridge: &Bridge) {
        let engine = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        // An allocate-like request and its answer.
        let request = stun(&[0, 0x19, 0, 4, 17, 0, 0, 0]);
        engine.send_to(&request, bridge.address()).await.unwrap();
        let (reply, from) = recv(&engine).await;
        assert_eq!(from, bridge.address(), "the engine sees one server address");
        assert_eq!(reply.len(), request.len());
        assert_eq!(reply[0], 0x01);
        // ChannelData with an unpadded length survives both directions.
        for payload in [&b"hello relay"[..], b"x", b"four"] {
            engine
                .send_to(&channel_data(payload), bridge.address())
                .await
                .unwrap();
            let (reply, _) = recv(&engine).await;
            assert_eq!(&reply[4..], payload.to_ascii_uppercase().as_slice());
            assert_eq!(reply.len(), 4 + payload.len());
        }
    }

    #[tokio::test]
    async fn plain_tcp_turn_is_reachable_through_a_loopback_udp_face() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            serve(stream).await;
        });
        let bridge = Bridge::start(
            Target {
                host: "127.0.0.1".into(),
                port,
                tls: false,
            },
            None,
        )
        .await
        .unwrap();
        exchange(&bridge).await;
    }

    fn fixtures() -> (CertificateDer<'static>, PrivateKeyDer<'static>) {
        (
            CertificateDer::from(include_bytes!("fixtures/turn-test-cert.der").to_vec()),
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
                include_bytes!("fixtures/turn-test-key.der").to_vec(),
            )),
        )
    }
    fn provider() -> Arc<rustls::crypto::CryptoProvider> {
        Arc::new(rustls::crypto::aws_lc_rs::default_provider())
    }
    async fn tls_server() -> u16 {
        let (certificate, key) = fixtures();
        let config = rustls::ServerConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(vec![certificate], key)
            .unwrap();
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((tcp, _)) = listener.accept().await {
                if let Ok(stream) = acceptor.accept(tcp).await {
                    tokio::spawn(serve(stream));
                }
            }
        });
        port
    }
    fn trusting(certificate: Option<CertificateDer<'static>>) -> Arc<rustls::ClientConfig> {
        let mut roots = rustls::RootCertStore::empty();
        if let Some(certificate) = certificate {
            roots.add(certificate).unwrap();
        }
        Arc::new(
            rustls::ClientConfig::builder_with_provider(provider())
                .with_safe_default_protocol_versions()
                .unwrap()
                .with_root_certificates(roots)
                .with_no_client_auth(),
        )
    }

    #[tokio::test]
    async fn tls_turn_is_reachable_when_the_certificate_is_trusted() {
        let port = tls_server().await;
        let bridge = Bridge::start(
            Target {
                host: "localhost".into(),
                port,
                tls: true,
            },
            Some(trusting(Some(fixtures().0))),
        )
        .await
        .unwrap();
        exchange(&bridge).await;
    }

    #[tokio::test]
    async fn an_untrusted_certificate_carries_no_traffic_at_all() {
        let port = tls_server().await;
        for host in ["localhost", "127.0.0.1"] {
            let bridge = Bridge::start(
                Target {
                    host: host.into(),
                    port,
                    tls: true,
                },
                Some(trusting(None)),
            )
            .await
            .unwrap();
            let engine = UdpSocket::bind("127.0.0.1:0").await.unwrap();
            engine
                .send_to(&stun(&[0, 0x19, 0, 4, 17, 0, 0, 0]), bridge.address())
                .await
                .unwrap();
            let mut buffer = [0_u8; 256];
            assert!(
                tokio::time::timeout(Duration::from_millis(800), engine.recv_from(&mut buffer))
                    .await
                    .is_err(),
                "{host}: no reply through an unverified connection"
            );
        }
    }

    #[tokio::test]
    async fn a_name_the_certificate_does_not_cover_is_refused() {
        let port = tls_server().await;
        let bridge = Bridge::start(
            Target {
                host: "turn.invalid".into(),
                port,
                tls: true,
            },
            Some(trusting(Some(fixtures().0))),
        )
        .await
        .unwrap();
        let engine = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        engine
            .send_to(&stun(&[0, 0x19, 0, 4, 17, 0, 0, 0]), bridge.address())
            .await
            .unwrap();
        let mut buffer = [0_u8; 256];
        assert!(
            tokio::time::timeout(Duration::from_millis(800), engine.recv_from(&mut buffer))
                .await
                .is_err()
        );
    }

    #[test]
    fn the_system_trust_store_configures() {
        assert!(system_tls().is_ok());
    }
}
