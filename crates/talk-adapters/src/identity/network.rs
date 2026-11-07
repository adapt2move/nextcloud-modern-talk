//! Cheap route observations on the identity runtime's background executor.
//! UDP connect chooses a route without sending a datagram; addresses remain in
//! memory and never enter settings or diagnostics. Both IP families are sampled.
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket};
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Fingerprint {
    v4: Option<IpAddr>,
    v6: Option<IpAddr>,
    routes: u64,
}
fn source(bind: SocketAddr, destination: SocketAddr) -> Option<IpAddr> {
    let socket = UdpSocket::bind(bind).ok()?;
    socket.connect(destination).ok()?;
    Some(socket.local_addr().ok()?.ip())
}
pub(super) fn fingerprint() -> Fingerprint {
    #[cfg(target_os = "linux")]
    let routes = {
        use std::io::Read;
        let mut bytes = Vec::with_capacity(4096);
        if let Ok(file) = std::fs::File::open("/proc/net/route") {
            let _ = file.take(16384).read_to_end(&mut bytes);
        }
        // Ignore changing route use/reference counters; only topology matters.
        let mut hash = 0xcbf29ce484222325u64;
        if let Ok(text) = std::str::from_utf8(&bytes) {
            for row in text.lines().skip(1).take(64) {
                for (column, value) in row.split_whitespace().enumerate() {
                    if matches!(column, 0 | 1 | 2 | 3 | 6 | 7) {
                        for byte in value.bytes().chain(std::iter::once(0)) {
                            hash = (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3);
                        }
                    }
                }
            }
        }
        hash
    };
    #[cfg(not(target_os = "linux"))]
    let routes = 0;
    Fingerprint {
        v4: source(
            SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)),
            SocketAddr::from((Ipv4Addr::new(198, 51, 100, 1), 9)),
        ),
        v6: source(
            SocketAddr::from((Ipv6Addr::UNSPECIFIED, 0)),
            SocketAddr::from((Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1), 9)),
        ),
        routes,
    }
}
