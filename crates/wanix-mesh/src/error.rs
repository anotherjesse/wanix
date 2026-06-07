//! Error types for the mesh transport edge.
//!
//! These wrap the failure modes that the iroh QUIC transport, the sync/async
//! bridge, and the 9P session can produce, keeping iroh's own error types out of
//! the crate's public surface where possible.

use std::fmt;

/// An error binding, dialing, or serving over the iroh mesh transport.
#[derive(Debug)]
pub enum MeshError {
    /// The local iroh [`crate::MeshNode`] endpoint could not be bound.
    Bind(String),
    /// Dialing the remote peer or opening the control stream failed.
    Dial(String),
    /// A QUIC stream operation (read, write, or finish) failed.
    Stream(String),
    /// The 9P client could not negotiate a session over the dialed stream.
    Session(String),
    /// A peer's verified identity could not be read from the connection.
    Identity(String),
}

impl fmt::Display for MeshError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bind(detail) => write!(f, "failed to bind mesh endpoint: {detail}"),
            Self::Dial(detail) => write!(f, "failed to dial mesh peer: {detail}"),
            Self::Stream(detail) => write!(f, "mesh stream error: {detail}"),
            Self::Session(detail) => write!(f, "failed to negotiate mesh 9P session: {detail}"),
            Self::Identity(detail) => write!(f, "could not read verified peer identity: {detail}"),
        }
    }
}

impl std::error::Error for MeshError {}

/// Result alias for fallible mesh transport operations.
pub type MeshResult<T> = Result<T, MeshError>;
