use std::net::SocketAddr;

use super::super::http::header_end;

pub(super) fn request_host(request: &[u8]) -> Option<String> {
    let header_end = header_end(request)?;
    let header = std::str::from_utf8(&request[..header_end]).ok()?;
    for line in header.lines().skip(1) {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case("host") {
            let host = value.trim();
            if is_safe_host(host) {
                return Some(host.to_owned());
            }
        }
    }
    None
}

fn is_safe_host(host: &str) -> bool {
    !host.is_empty()
        && host.bytes().all(|byte| {
            matches!(
                byte,
                b'a'..=b'z'
                    | b'A'..=b'Z'
                    | b'0'..=b'9'
                    | b'.'
                    | b'-'
                    | b'_'
                    | b':'
                    | b'['
                    | b']'
            )
        })
}

pub(super) fn is_loopback_peer(peer_addr: SocketAddr) -> bool {
    peer_addr.ip().is_loopback()
}

pub(in crate::serve) fn display_host(local_addr: SocketAddr) -> String {
    let host = if local_addr.ip().is_unspecified() {
        "localhost".to_owned()
    } else if local_addr.ip().is_ipv6() {
        format!("[{}]", local_addr.ip())
    } else {
        local_addr.ip().to_string()
    };
    format!("{host}:{}", local_addr.port())
}
