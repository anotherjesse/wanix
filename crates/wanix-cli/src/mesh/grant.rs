//! Grant-spec parsing for `mesh-serve`, mirroring the `p9-listen` grant surface.
//!
//! A `--grant ANAME:PREFIX:RIGHTS` spec names a capability: the verified peer
//! (from `--peer`) may attach `ANAME` and receive the served root re-rooted at
//! `PREFIX` with `RIGHTS` (`rw` or `ro`). The table is default-deny, so a peer
//! with no matching grant is rejected at attach.

use std::sync::Arc;

use wanix_fs::FileSystem;
use wanix_id::{Grant, GrantTable, PeerId};
use wanix_vfs::Rights;

use crate::CliError;

/// A parsed `ANAME:PREFIX:RIGHTS` grant spec, not yet bound to a peer or backing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GrantSpec {
    aname: String,
    prefix: String,
    rights: Rights,
}

impl GrantSpec {
    /// Parses one `ANAME:PREFIX:RIGHTS` grant spec.
    ///
    /// # Errors
    ///
    /// Returns a usage error when the spec is not exactly three colon-separated
    /// fields or the rights token is not `rw` or `ro`.
    pub(crate) fn parse(spec: &str) -> Result<Self, CliError> {
        let mut fields = spec.split(':');
        let aname = fields.next().unwrap_or_default();
        let prefix = fields.next();
        let rights = fields.next();
        if aname.is_empty() || prefix.is_none() || rights.is_none() || fields.next().is_some() {
            return Err(CliError::usage(format!(
                "mesh-serve --grant expects ANAME:PREFIX:RIGHTS, got {spec:?}"
            )));
        }
        let rights = match rights.unwrap_or_default() {
            "rw" => Rights::read_write(),
            "ro" => Rights::read_only(),
            other => {
                return Err(CliError::usage(format!(
                    "mesh-serve --grant RIGHTS must be rw or ro, got {other:?}"
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
pub(crate) fn parse_peer_hex(value: &str) -> Result<PeerId, CliError> {
    if value.len() != 64 {
        return Err(CliError::usage(format!(
            "mesh-serve --peer expects 64 hex digits, got {} characters",
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
            "mesh-serve --peer has a non-hex digit {:?}",
            other as char
        ))),
    }
}

/// Fills `table` with grants for `peer`, all scoped to `backing` (the root).
pub(crate) fn build_grant_table(
    table: &GrantTable,
    peer: PeerId,
    specs: &[GrantSpec],
    backing: Arc<dyn FileSystem>,
) {
    for spec in specs {
        table.grant(spec.clone().into_grant(peer, Arc::clone(&backing)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_grant_spec_reads_three_fields() {
        let spec = GrantSpec::parse("projects/foo:projects/foo:rw").unwrap();
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
    fn parse_peer_hex_round_trips() {
        let ones = "01".repeat(32);
        assert_eq!(parse_peer_hex(&ones).unwrap().as_bytes(), &[1u8; 32]);
        assert!(parse_peer_hex("short").is_err());
        assert!(parse_peer_hex(&"z".repeat(64)).is_err());
    }
}
