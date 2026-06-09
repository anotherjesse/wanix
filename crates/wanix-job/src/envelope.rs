//! The shared `"wanix.resource"` spec envelope (ADR 0009 §Discovery,
//! `docs/toolfs.md` §"Spec Shape").
//!
//! Every self-describing resource — ToolFS (`"kind": "tool"`), AppResource
//! (`"kind": "app"`), catalog entries embedding a summary — opens its
//! `spec.json` with one versioned outer shape, so agents parse one envelope
//! everywhere. Devices `#[serde(flatten)]` this struct into their own spec
//! type and add their device-specific fields (input/outputs/limits/...)
//! alongside it.

use serde::{Deserialize, Serialize};

/// The literal JSON key that marks a self-describing Wanix resource spec.
pub const WANIX_RESOURCE_KEY: &str = "wanix.resource";

/// The version tag carried under the [`WANIX_RESOURCE_KEY`] key.
///
/// Deserialization rejects any value other than a declared version, so a
/// future `"v1"` spec fails loudly in a `v0`-only client instead of being
/// half-read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ResourceEnvelopeVersion {
    /// The first (current) envelope shape.
    #[default]
    #[serde(rename = "v0")]
    V0,
}

/// The shared resource envelope: version marker plus identity prose.
///
/// This is the outer shape only; everything after `description` in a spec
/// (input/params/outputs/limits/lifecycle/effects) belongs to the device's
/// own spec struct, which embeds this with `#[serde(flatten)]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceEnvelope {
    /// Envelope version, serialized under the literal `"wanix.resource"` key.
    #[serde(rename = "wanix.resource")]
    pub version: ResourceEnvelopeVersion,
    /// What sort of resource this is (`"tool"`, `"app"`, ...). Open set.
    pub kind: String,
    /// The resource's name, as cataloged and mounted.
    pub name: String,
    /// One-line human description.
    pub description: String,
}

impl ResourceEnvelope {
    /// Build a `v0` envelope.
    pub fn v0(
        kind: impl Into<String>,
        name: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            version: ResourceEnvelopeVersion::V0,
            kind: kind.into(),
            name: name.into(),
            description: description.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ResourceEnvelope, ResourceEnvelopeVersion, WANIX_RESOURCE_KEY};
    use serde::{Deserialize, Serialize};
    use serde_json::json;

    #[test]
    fn envelope_pins_the_literal_key_and_version() {
        let envelope = ResourceEnvelope::v0("tool", "upper", "Uppercase UTF-8 text.");
        let value = serde_json::to_value(&envelope).unwrap();
        assert_eq!(value[WANIX_RESOURCE_KEY], json!("v0"));
        assert_eq!(
            value,
            json!({
                "wanix.resource": "v0",
                "kind": "tool",
                "name": "upper",
                "description": "Uppercase UTF-8 text."
            })
        );
        let back: ResourceEnvelope = serde_json::from_value(value).unwrap();
        assert_eq!(back, envelope);
    }

    #[test]
    fn unknown_versions_are_rejected() {
        let err = serde_json::from_value::<ResourceEnvelope>(json!({
            "wanix.resource": "v1",
            "kind": "tool",
            "name": "upper",
            "description": "..."
        }));
        assert!(err.is_err());
        assert!(serde_json::from_value::<ResourceEnvelopeVersion>(json!("v0")).is_ok());
    }

    #[test]
    fn envelope_flattens_into_a_device_spec() {
        // The intended embedding: a device spec carries the envelope at its
        // top level, exactly like the docs/toolfs.md spec sketch opens.
        #[derive(Serialize, Deserialize)]
        struct ToySpec {
            #[serde(flatten)]
            envelope: ResourceEnvelope,
            retryable: bool,
        }

        let spec = ToySpec {
            envelope: ResourceEnvelope::v0("tool", "upper", "Uppercase UTF-8 text."),
            retryable: true,
        };
        let value = serde_json::to_value(&spec).unwrap();
        assert_eq!(
            value,
            json!({
                "wanix.resource": "v0",
                "kind": "tool",
                "name": "upper",
                "description": "Uppercase UTF-8 text.",
                "retryable": true
            })
        );
        let back: ToySpec = serde_json::from_value(value).unwrap();
        assert_eq!(back.envelope, spec.envelope);
        assert!(back.retryable);
    }
}
