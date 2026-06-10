//! The newline-JSON file2chan wire protocol between adapter and guest.
//!
//! One JSON object per line. The adapter sends [`AppRequest`] lines; the
//! guest answers with [`AppReply`] lines and may interleave guest-initiated
//! [`AppPublish`] lines (`{"publish":{...}}`) between replies. Arbitrary
//! bytes ride as base64 strings. Error kinds mirror [`FsError`] variants
//! one-to-one — this is filesystem-error vocabulary, not the ADR 0009 job
//! taxonomy. Every field is pinned by tests in `src/tests.rs`.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::{Deserialize, Serialize};
use wanix_fs::{FsError, FsResult};

/// Hard ceiling on one encoded protocol line, enforced on both directions.
///
/// Bounds the allocation a guest (or a client write routed to the guest) can
/// force through the channel. Matches the per-subscriber stream buffer
/// ceiling so one publish can at most fill one subscriber buffer.
pub const MAX_LINE_LEN: usize = 1024 * 1024;

/// Encodes arbitrary bytes as the protocol's base64 `data` string.
#[must_use]
pub fn encode_data(bytes: &[u8]) -> String {
    BASE64.encode(bytes)
}

/// Decodes a protocol base64 `data` string back to bytes.
///
/// # Errors
///
/// Returns [`FsError::Other`] when the string is not valid base64.
pub fn decode_data(data: &str) -> FsResult<Vec<u8>> {
    BASE64
        .decode(data)
        .map_err(|err| FsError::Other(format!("invalid base64 data in app protocol: {err}")))
}

/// The discrete filesystem operations routed to the guest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AppOp {
    /// Read the whole content of a guest-handled file.
    Read,
    /// Write one message/body to a guest-handled file (`data` is base64).
    Write,
    /// List the entries of a guest-handled path.
    Readdir,
    /// Confirm a guest-handled path exists.
    Stat,
}

/// One adapter→guest operation event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppRequest {
    /// Operation id; unique and increasing per adapter, echoed by the reply.
    pub id: u64,
    /// The operation kind.
    pub op: AppOp,
    /// The guest-handled path, relative to the app root (no leading slash).
    pub path: String,
    /// The verified acting principal (opaque to this crate; comes from the
    /// transport/attach layer, never from a client payload).
    pub principal: String,
    /// Base64-encoded payload bytes; present only for `write`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
}

impl AppRequest {
    /// Serializes the request to one newline-terminated JSON line.
    ///
    /// # Errors
    ///
    /// Returns [`FsError::Other`] when the encoded line exceeds
    /// [`MAX_LINE_LEN`] (an oversized client write) or fails to encode.
    pub fn to_line(&self) -> FsResult<Vec<u8>> {
        let mut line = serde_json::to_vec(self)
            .map_err(|err| FsError::Other(format!("failed to encode app request: {err}")))?;
        line.push(b'\n');
        if line.len() > MAX_LINE_LEN {
            return Err(FsError::Other(format!(
                "app request line of {} bytes exceeds the {MAX_LINE_LEN}-byte ceiling",
                line.len()
            )));
        }
        Ok(line)
    }
}

/// One entry in a guest `readdir` reply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppDirEntry {
    /// Entry basename.
    pub name: String,
    /// Whether the entry is a directory (defaults to a file).
    #[serde(default)]
    pub dir: bool,
}

/// The success payload of a reply: `{}`, `{"data":...}`, or `{"entries":...}`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppOk {
    /// Base64-encoded content bytes (for `read`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
    /// Directory entries (for `readdir`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entries: Option<Vec<AppDirEntry>>,
}

impl AppOk {
    /// Decodes the `data` field, treating an absent field as empty bytes.
    ///
    /// # Errors
    ///
    /// Returns [`FsError::Other`] on invalid base64.
    pub fn data_bytes(&self) -> FsResult<Vec<u8>> {
        self.data.as_deref().map_or(Ok(Vec::new()), decode_data)
    }
}

/// Guest error kinds; each maps onto exactly one [`FsError`] variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppErrKind {
    /// The path does not exist → [`FsError::NotFound`].
    NotFound,
    /// The caller may not do this → [`FsError::PermissionDenied`].
    PermissionDenied,
    /// The app does not support this op here → [`FsError::NotSupported`].
    NotSupported,
    /// The request was malformed for this app → [`FsError::InvalidPath`].
    Invalid,
    /// Anything else, with a diagnostic message → [`FsError::Other`].
    Other,
}

/// The error payload of a reply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppErr {
    /// The error kind (see [`AppErrKind`] for the `FsError` mapping).
    pub kind: AppErrKind,
    /// Optional human-readable detail.
    #[serde(default)]
    pub message: String,
}

impl AppErr {
    /// Maps the guest error onto its [`FsError`] variant.
    #[must_use]
    pub fn to_fs_error(&self) -> FsError {
        match self.kind {
            AppErrKind::NotFound => FsError::NotFound,
            AppErrKind::PermissionDenied => FsError::PermissionDenied,
            AppErrKind::NotSupported => FsError::NotSupported,
            AppErrKind::Invalid => FsError::InvalidPath(self.message.clone()),
            AppErrKind::Other => FsError::Other(self.message.clone()),
        }
    }
}

/// One guest→adapter reply: `{"id":N,"ok":{...}}` or `{"id":N,"err":{...}}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppReply {
    /// The id of the request this reply answers.
    pub id: u64,
    /// Success payload; mutually exclusive with `err`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ok: Option<AppOk>,
    /// Error payload; mutually exclusive with `ok`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub err: Option<AppErr>,
}

impl AppReply {
    /// Resolves the reply into its success payload or mapped error.
    ///
    /// # Errors
    ///
    /// Returns the guest's mapped [`FsError`] for an `err` reply, or
    /// [`FsError::Other`] when the reply carries both or neither payload.
    pub fn into_result(self) -> FsResult<AppOk> {
        match (self.ok, self.err) {
            (Some(ok), None) => Ok(ok),
            (None, Some(err)) => Err(err.to_fs_error()),
            _ => Err(FsError::Other(
                "app reply must carry exactly one of ok/err".to_owned(),
            )),
        }
    }
}

/// A guest-initiated stream publish: `{"publish":{"stream":...,"data":...}}`.
///
/// May arrive between replies; the host appends the decoded bytes to every
/// subscriber buffer of the declared stream file. The guest is responsible
/// for its own line framing inside `data` (e.g. newline-JSON messages).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppPublish {
    /// The declared stream path the bytes belong to.
    pub stream: String,
    /// Base64-encoded bytes to append to every subscriber buffer.
    pub data: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PublishLine {
    publish: AppPublish,
}

/// One parsed guest→adapter line: a reply or a publish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuestLine {
    /// A reply to an in-flight request.
    Reply(AppReply),
    /// A guest-initiated stream publish.
    Publish(AppPublish),
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawGuestLine {
    Publish(PublishLine),
    Reply(AppReply),
}

impl GuestLine {
    /// Parses one guest line (a trailing newline is tolerated).
    ///
    /// # Errors
    ///
    /// Returns [`FsError::Other`] when the line exceeds [`MAX_LINE_LEN`] or
    /// is not exactly one reply or publish object.
    pub fn parse(line: &[u8]) -> FsResult<Self> {
        if line.len() > MAX_LINE_LEN {
            return Err(FsError::Other(format!(
                "app guest line of {} bytes exceeds the {MAX_LINE_LEN}-byte ceiling",
                line.len()
            )));
        }
        let trimmed = line.strip_suffix(b"\n").unwrap_or(line);
        match serde_json::from_slice(trimmed) {
            Ok(RawGuestLine::Publish(line)) => Ok(Self::Publish(line.publish)),
            Ok(RawGuestLine::Reply(reply)) => Ok(Self::Reply(reply)),
            Err(err) => Err(FsError::Other(format!("invalid app guest line: {err}"))),
        }
    }
}

/// Serializes a publish to its wire line (used by guest-side helpers/tests).
///
/// # Errors
///
/// Returns [`FsError::Other`] when encoding fails or the line exceeds
/// [`MAX_LINE_LEN`].
pub fn publish_to_line(publish: &AppPublish) -> FsResult<Vec<u8>> {
    let mut line = serde_json::to_vec(&PublishLine {
        publish: publish.clone(),
    })
    .map_err(|err| FsError::Other(format!("failed to encode app publish: {err}")))?;
    line.push(b'\n');
    if line.len() > MAX_LINE_LEN {
        return Err(FsError::Other(format!(
            "app publish line of {} bytes exceeds the {MAX_LINE_LEN}-byte ceiling",
            line.len()
        )));
    }
    Ok(line)
}
