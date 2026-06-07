//! The blocking 9P request/response connection state machine.
//!
//! [`P9Conn`] owns the [`Duplex`] transport, the negotiated `msize` and Google
//! protocol version, and the [`FidPool`]/[`TagPool`] allocators. Its [`rpc`]
//! primitive is the mirror of the server's `serve_stream` loop: it encodes one
//! T-message, blocking-reads R-message frames until the matching tag returns,
//! and surfaces an `Rlerror` as a typed [`ClientError::Remote`].
//!
//! Because the server is strictly serial, the client keeps exactly one request
//! outstanding. [`crate::RemoteFs`] wraps the connection in `Arc<Mutex<_>>`, so
//! concurrent callers serialize on the mutex rather than racing on the wire.
//!
//! [`rpc`]: P9Conn::rpc

use wanix_protocol::{
    P9_NOFID, P9_RLERROR, P9_VERSION_9P2000_L_GOOGLE_2, P9Frame, P9Qid, p9_decode_rattach,
    p9_decode_rlerror, p9_decode_rversion, p9_tattach, p9_tclunk, p9_tversion,
};

use crate::error::{ClientError, ClientResult};
use crate::fid::{FidPool, TagPool};
use crate::transport::{Duplex, read_one_frame, write_frame};

/// The fid bound to the attached root of the served tree.
pub const ROOT_FID: u32 = 0;

/// Google.2 protocol level that unlocks `Twalkgetattr` batching.
pub const GOOGLE_WALKGETATTR_VERSION: u32 = 2;

/// The client's preferred maximum 9P message size offered during negotiation.
const PREFERRED_MSIZE: u32 = 131_072;

/// A blocking 9P client connection over one [`Duplex`] transport.
pub struct P9Conn {
    transport: Box<dyn Duplex>,
    msize: u32,
    google_version: u32,
    fids: FidPool,
    tags: TagPool,
    poisoned: Option<String>,
}

impl P9Conn {
    /// Negotiates `Tversion` then `Tattach` over `transport`, returning a ready
    /// connection bound to the served root at [`ROOT_FID`].
    ///
    /// The client offers `9P2000.L.Google.2` first so the server can enable
    /// `Twalkgetattr` batching; a server that only speaks base `9P2000.L`
    /// negotiates down transparently and `google_version` reports `0`.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] when negotiation I/O fails, a reply is malformed,
    /// or the server rejects the attach with an `Rlerror`.
    pub fn connect(transport: Box<dyn Duplex>) -> ClientResult<Self> {
        Self::connect_with_aname(transport, "")
    }

    /// Negotiates a session and attaches the named subtree `aname`.
    ///
    /// This is the mesh-facing constructor: a server gated by an
    /// [`crate::ClientError`]-mapped `AttachPolicy` keys its grant by the verified
    /// peer identity *and* the requested attach name, so a client importing a
    /// scoped capability must send the matching `aname` (e.g. `projects/foo`).
    /// [`Self::connect`] sends the empty root `aname` for the unscoped case.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] when negotiation I/O fails, a reply is malformed,
    /// or the server rejects the attach with an `Rlerror` (for example a
    /// default-deny grant table returning `EACCES`).
    pub fn connect_with_aname(transport: Box<dyn Duplex>, aname: &str) -> ClientResult<Self> {
        let mut conn = Self {
            transport,
            msize: PREFERRED_MSIZE,
            google_version: 0,
            fids: FidPool::new(),
            tags: TagPool::new(),
            poisoned: None,
        };
        conn.negotiate_version()?;
        conn.attach_root(aname)?;
        Ok(conn)
    }

    /// Returns the negotiated maximum 9P message size.
    #[must_use]
    pub fn msize(&self) -> u32 {
        self.msize
    }

    /// Returns the negotiated Google protocol version (`0` for base 9P2000.L).
    #[must_use]
    pub fn google_version(&self) -> u32 {
        self.google_version
    }

    /// Returns whether the server speaks the `Twalkgetattr` Google extension.
    #[must_use]
    pub fn supports_walkgetattr(&self) -> bool {
        self.google_version >= GOOGLE_WALKGETATTR_VERSION
    }

    /// Allocates a fresh fid number, failing if the connection is poisoned.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::Poisoned`] when the connection is dead or the fid
    /// space is exhausted.
    pub fn allocate_fid(&mut self) -> ClientResult<u32> {
        self.check_poison()?;
        self.fids
            .allocate()
            .ok_or_else(|| ClientError::Poisoned("9P fid space exhausted".to_owned()))
    }

    /// Returns `fid` to the pool for reuse.
    pub fn free_fid(&mut self, fid: u32) {
        self.fids.release(fid);
    }

    /// Best-effort releases `fid` on the server and returns it to the pool.
    ///
    /// Used by [`crate::fid::ScratchFid`] and [`crate::file::RemoteFile`] drop
    /// paths: a clunk that fails on a dying handle cannot be retried, so the
    /// error is swallowed after the fid is reclaimed locally.
    pub fn clunk_fid(&mut self, fid: u32) {
        if self.poisoned.is_none() {
            let _ = self.rpc(|tag| Ok(p9_tclunk(tag, fid)));
        }
        self.fids.release(fid);
    }

    /// Performs one blocking request/response exchange.
    ///
    /// `build` is called with a freshly allocated tag and returns the encoded
    /// T-message frame. The reply is read with the negotiated `msize` ceiling
    /// enforced; an `Rlerror` becomes [`ClientError::Remote`] and a tag mismatch
    /// poisons the connection.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on transport failure, malformed reply, server
    /// `Rlerror`, tag mismatch, or a poisoned connection.
    pub fn rpc<F>(&mut self, build: F) -> ClientResult<P9Frame>
    where
        F: FnOnce(u16) -> Result<P9Frame, wanix_protocol::P9Error>,
    {
        self.check_poison()?;
        let tag = self
            .tags
            .allocate()
            .ok_or_else(|| ClientError::Poisoned("9P tag space exhausted".to_owned()))?;
        let result = self.rpc_with_tag(tag, build);
        self.tags.release(tag);
        result
    }

    fn rpc_with_tag<F>(&mut self, tag: u16, build: F) -> ClientResult<P9Frame>
    where
        F: FnOnce(u16) -> Result<P9Frame, wanix_protocol::P9Error>,
    {
        let request = build(tag)?;
        if let Err(error) = write_frame(&mut self.transport, &request) {
            return Err(self.poison(error));
        }
        let reply = match read_one_frame(&mut self.transport, self.msize) {
            Ok(reply) => reply,
            Err(error) => return Err(self.poison(error)),
        };
        if reply.tag() != tag {
            let reason = format!(
                "server replied with tag {} to request tag {tag}",
                reply.tag()
            );
            return Err(self.poison(ClientError::Poisoned(reason)));
        }
        if reply.message_type() == P9_RLERROR {
            let lerror = p9_decode_rlerror(&reply)?;
            return Err(ClientError::Remote {
                errno: lerror.ecode,
            });
        }
        Ok(reply)
    }

    fn negotiate_version(&mut self) -> ClientResult<()> {
        let reply =
            self.rpc(|tag| p9_tversion(tag, PREFERRED_MSIZE, P9_VERSION_9P2000_L_GOOGLE_2))?;
        let version = p9_decode_rversion(&reply)?;
        self.msize = version.msize.clamp(P9_HEADER_FLOOR, PREFERRED_MSIZE);
        self.google_version = google_version_for(&version.version);
        Ok(())
    }

    fn attach_root(&mut self, aname: &str) -> ClientResult<P9Qid> {
        let reply = self.rpc(|tag| p9_tattach(tag, ROOT_FID, P9_NOFID, "wanix", aname, 0))?;
        Ok(p9_decode_rattach(&reply)?)
    }

    fn check_poison(&self) -> ClientResult<()> {
        match &self.poisoned {
            Some(reason) => Err(ClientError::Poisoned(reason.clone())),
            None => Ok(()),
        }
    }

    fn poison(&mut self, error: ClientError) -> ClientError {
        let reason = error.to_string();
        if self.poisoned.is_none() {
            self.poisoned = Some(reason);
        }
        error
    }
}

/// Smallest `msize` the client will accept; a server may not negotiate below a
/// usable frame size.
const P9_HEADER_FLOOR: u32 = 512;

/// Maps a negotiated version string onto the Google extension level.
///
/// Only the `9P2000.L.Google.2` string unlocks `Twalkgetattr`; every other
/// accepted version (including base `9P2000.L`) reports level `0`.
fn google_version_for(version: &str) -> u32 {
    if version == P9_VERSION_9P2000_L_GOOGLE_2 {
        GOOGLE_WALKGETATTR_VERSION
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use wanix_protocol::P9_VERSION_9P2000_L;

    use super::*;

    #[test]
    fn google_version_detects_walkgetattr_level() {
        assert_eq!(
            google_version_for(P9_VERSION_9P2000_L_GOOGLE_2),
            GOOGLE_WALKGETATTR_VERSION
        );
        assert_eq!(google_version_for(P9_VERSION_9P2000_L), 0);
        assert_eq!(google_version_for("9P2000.L.Google.1"), 0);
    }
}
