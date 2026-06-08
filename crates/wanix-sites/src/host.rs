//! The [`Host`] newtype: a normalized site hostname.

use std::fmt;

/// A normalized site host: lowercased, with any `:port` suffix stripped.
///
/// `#sites` binds a host to a filesystem source, and the HTTP gateway looks a
/// request up by its `Host` header. Both go through this newtype so the binding
/// key is always canonical — `Blog.LocalHost:7654` and `blog.localhost` resolve
/// to the same site.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Host(String);

impl Host {
    /// Parses and normalizes a raw host string (e.g. an HTTP `Host` header
    /// value), stripping a trailing `:port` and lowercasing.
    ///
    /// Returns `None` when the host is empty, an IP literal, or the bare
    /// `localhost` — those fall through to the gateway's `--root` behavior
    /// rather than naming a site.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        let host = strip_port(raw.trim()).trim();
        if host.is_empty() || is_bare_or_ip(host) {
            return None;
        }
        Some(Self(host.to_ascii_lowercase()))
    }

    /// Builds a `Host` from an already-trusted name without the gateway's
    /// bare-host/IP filtering, used to register a site programmatically (serve
    /// startup, tests). Still lowercases and strips a port for canonical keys;
    /// returns `None` only for an empty name.
    #[must_use]
    pub fn registered(raw: &str) -> Option<Self> {
        let host = strip_port(raw.trim()).trim();
        if host.is_empty() {
            return None;
        }
        Some(Self(host.to_ascii_lowercase()))
    }

    /// Returns the normalized host string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Host {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Strips a trailing `:port`, leaving bracketed IPv6 literals (`[::1]:80`)
/// intact up to and including their closing bracket.
fn strip_port(host: &str) -> &str {
    if host.starts_with('[') {
        // IPv6 literal: keep through the closing bracket, drop any `:port`.
        return match host.find(']') {
            Some(close) => &host[..=close],
            None => host,
        };
    }
    match host.rsplit_once(':') {
        Some((name, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => name,
        _ => host,
    }
}

/// A host names a site only when it is a real (multi-label or non-`localhost`)
/// name. Bare `localhost`, an IPv4 literal, or an IPv6 literal fall through.
fn is_bare_or_ip(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host.starts_with('[')
        || host.bytes().all(|b| b.is_ascii_digit() || b == b'.')
}
