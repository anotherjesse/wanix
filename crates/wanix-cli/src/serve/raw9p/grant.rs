use std::sync::Arc;

use wanix_9p::{Grant, GrantTable, GrantTablePolicy, PeerId};
use wanix_fs::FileSystem;
use wanix_vfs::Rights;

use crate::CliError;

/// A parsed `--grant ANAME:PREFIX:RIGHTS` capability spec, not yet bound to a
/// peer or backing filesystem.
///
/// The peer is supplied once via `--peer` (explicit identity, since there is no
/// QUIC handshake over plain TCP to verify it), and the backing filesystem is
/// the serve `--p9` root. This keeps the wire format of a grant — who, which
/// subtree, which rights — out of the transport code and in one auditable
/// place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::serve) struct GrantSpec {
    aname: String,
    prefix: String,
    rights: Rights,
}

impl GrantSpec {
    /// Parses one `ANAME:PREFIX:RIGHTS` grant spec.
    ///
    /// `RIGHTS` is `rw` (read-write) or `ro` (read-only). `ANAME` and `PREFIX`
    /// are Wanix paths and may contain `/` but not `:`.
    ///
    /// # Errors
    ///
    /// Returns a usage error when the spec is not exactly three colon-separated
    /// fields or the rights token is unknown.
    pub(in crate::serve) fn parse(spec: &str) -> Result<Self, CliError> {
        let mut fields = spec.split(':');
        let aname = fields.next().unwrap_or_default();
        let prefix = fields.next();
        let rights = fields.next();
        if aname.is_empty() || prefix.is_none() || rights.is_none() || fields.next().is_some() {
            return Err(CliError::usage(format!(
                "serve --grant expects ANAME:PREFIX:RIGHTS, got {spec:?}"
            )));
        }
        let rights = match rights.unwrap_or_default() {
            "rw" => Rights::read_write(),
            "ro" => Rights::read_only(),
            other => {
                return Err(CliError::usage(format!(
                    "serve --grant RIGHTS must be rw or ro, got {other:?}"
                )));
            }
        };
        Ok(Self {
            aname: aname.to_owned(),
            prefix: prefix.unwrap_or_default().to_owned(),
            rights,
        })
    }

    /// Binds this spec to `peer` over `backing`, producing a [`Grant`].
    fn into_grant(self, peer: PeerId, backing: Arc<dyn FileSystem>) -> Grant {
        Grant::new(peer, self.aname, backing, self.prefix, self.rights)
    }
}

/// Parses a 64-character lowercase-hex ed25519 public key into a [`PeerId`].
///
/// # Errors
///
/// Returns a usage error when the value is not exactly 64 hex digits.
pub(in crate::serve) fn parse_peer(value: &str) -> Result<PeerId, CliError> {
    if value.len() != 64 {
        return Err(CliError::usage(format!(
            "serve --peer expects 64 hex digits, got {} characters",
            value.len()
        )));
    }
    let mut bytes = [0u8; 32];
    for (index, chunk) in value.as_bytes().chunks_exact(2).enumerate() {
        let hi = hex_value(chunk[0])?;
        let lo = hex_value(chunk[1])?;
        bytes[index] = (hi << 4) | lo;
    }
    Ok(PeerId::from_bytes(bytes))
}

fn hex_value(byte: u8) -> Result<u8, CliError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        other => Err(CliError::usage(format!(
            "serve --peer has a non-hex digit {:?}",
            other as char
        ))),
    }
}

/// Builds a default-deny [`GrantTablePolicy`] from a peer and its grant specs,
/// all scoped to `backing` (the serve `--p9` root).
///
/// The returned table is shared (cheap clone), so it remains live-editable: the
/// same table can be revoked from while the policy reads it on each attach.
pub(in crate::serve) fn build_policy(
    peer: PeerId,
    specs: Vec<GrantSpec>,
    backing: Arc<dyn FileSystem>,
) -> (GrantTable, GrantTablePolicy) {
    let table = GrantTable::new();
    for spec in specs {
        table.grant(spec.into_grant(peer, Arc::clone(&backing)));
    }
    let policy = GrantTablePolicy::new(table.clone());
    (table, policy)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_grant_spec_reads_three_fields() {
        let spec = GrantSpec::parse("projects/foo:projects/foo:rw").unwrap();
        assert_eq!(spec.aname, "projects/foo");
        assert_eq!(spec.prefix, "projects/foo");
        assert_eq!(spec.rights, Rights::read_write());

        let ro = GrantSpec::parse("docs:docs:ro").unwrap();
        assert_eq!(ro.rights, Rights::read_only());
    }

    #[test]
    fn parse_grant_spec_rejects_malformed_specs() {
        assert!(GrantSpec::parse("only-two:fields").is_err());
        assert!(GrantSpec::parse("a:b:c:d").is_err());
        assert!(GrantSpec::parse(":b:rw").is_err());
        assert!(GrantSpec::parse("a:b:wx").is_err());
    }

    #[test]
    fn parse_peer_round_trips_hex() {
        let hex = "0".repeat(64);
        assert_eq!(parse_peer(&hex).unwrap().as_bytes(), &[0u8; 32]);

        let ones = "01".repeat(32);
        assert_eq!(parse_peer(&ones).unwrap().as_bytes(), &[1u8; 32]);

        assert!(parse_peer("short").is_err());
        assert!(parse_peer(&"z".repeat(64)).is_err());
    }
}
