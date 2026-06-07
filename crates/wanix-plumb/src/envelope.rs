//! The newline-JSON message envelope carried on a `#plumb` topic.
//!
//! Every `#plumb` message is one JSON object on its own line:
//! `{"kind":..,"from":..,"to":..,"body":..}`. The fields mirror Plan 9's
//! plumber rules — `kind` is the typed routing tag (e.g. `task.done`), `from`
//! and `to` are optional addresses (a node id, an agent id, or empty for a
//! broadcast), and `body` is the free-form payload. Delivery is best-effort
//! epidemic gossip, **not** a durable queue: a subscriber that was not listening
//! when a message was broadcast never sees it, and there is no acknowledgement.
//! Durable handoff belongs in `#kv` or a content-addressed capsule blob.

use serde::{Deserialize, Serialize};

/// Upper bound on a single encoded envelope, enforced on both send and receive.
///
/// A `#plumb` message rides one gossip broadcast, which itself is size-bounded;
/// this ceiling rejects an oversized `send` write before it reaches the port and
/// bounds the buffer a `recv` reader accumulates per line, so a hostile peer
/// cannot force an unbounded allocation through the bus.
pub const MAX_ENVELOPE_LEN: usize = 64 * 1024;

/// A typed, addressed `#plumb` message: the unit of coordination on the bus.
///
/// Constructed from a `send` write (parsed as JSON) and serialized back to one
/// newline-terminated JSON line for a `recv` reader. Unknown JSON fields are
/// rejected so a typo in a `send` payload surfaces as an error rather than being
/// silently dropped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlumbEnvelope {
    /// The typed routing tag (e.g. `task.done`), the primary dispatch key.
    pub kind: String,
    /// The sender's address (a node or agent id), or empty when anonymous.
    #[serde(default)]
    pub from: String,
    /// The intended recipient's address, or empty for a topic-wide broadcast.
    #[serde(default)]
    pub to: String,
    /// The free-form message payload, an arbitrary JSON value.
    #[serde(default)]
    pub body: serde_json::Value,
}

impl PlumbEnvelope {
    /// Builds an envelope with the given `kind` and an empty `from`/`to` and a
    /// null `body` — the minimal broadcast.
    #[must_use]
    pub fn new(kind: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            from: String::new(),
            to: String::new(),
            body: serde_json::Value::Null,
        }
    }

    /// Parses one envelope from a `send` write's JSON bytes.
    ///
    /// # Errors
    ///
    /// Returns a message describing the failure when the bytes are not a single
    /// JSON object matching the envelope shape, or exceed [`MAX_ENVELOPE_LEN`].
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_ENVELOPE_LEN {
            return Err(format!(
                "plumb envelope of {} bytes exceeds the {MAX_ENVELOPE_LEN}-byte ceiling",
                bytes.len()
            ));
        }
        serde_json::from_slice(bytes).map_err(|err| format!("invalid plumb envelope: {err}"))
    }

    /// Serializes the envelope to one newline-terminated JSON line for `recv`.
    ///
    /// The trailing newline is what makes the `recv` stream line-delimited, so a
    /// reader can split received envelopes without a length prefix.
    ///
    /// # Errors
    ///
    /// Returns a message when serialization fails (only on a non-serializable
    /// `body`, which the `Value` type prevents in practice).
    pub fn to_line(&self) -> Result<Vec<u8>, String> {
        let mut line = serde_json::to_vec(self)
            .map_err(|err| format!("failed to encode plumb envelope: {err}"))?;
        line.push(b'\n');
        Ok(line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_a_full_envelope() {
        let env = PlumbEnvelope {
            kind: "task.done".to_owned(),
            from: "nodeA".to_owned(),
            to: "nodeB".to_owned(),
            body: serde_json::json!({ "path": "/world/out" }),
        };
        let line = env.to_line().unwrap();
        assert_eq!(*line.last().unwrap(), b'\n');
        let parsed = PlumbEnvelope::parse(&line[..line.len() - 1]).unwrap();
        assert_eq!(parsed, env);
    }

    #[test]
    fn defaults_fill_missing_fields() {
        let env = PlumbEnvelope::parse(br#"{"kind":"ping"}"#).unwrap();
        assert_eq!(env.kind, "ping");
        assert!(env.from.is_empty());
        assert!(env.to.is_empty());
        assert_eq!(env.body, serde_json::Value::Null);
    }

    #[test]
    fn rejects_unknown_fields() {
        assert!(PlumbEnvelope::parse(br#"{"kind":"x","oops":1}"#).is_err());
    }

    #[test]
    fn rejects_oversized_envelope() {
        let big = vec![b'a'; MAX_ENVELOPE_LEN + 1];
        assert!(PlumbEnvelope::parse(&big).is_err());
    }
}
