use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use ed25519_dalek::{SigningKey, VerifyingKey};

use crate::PeerId;

/// Length in bytes of an ed25519 secret key (the persisted seed).
pub(crate) const SECRET_KEY_LEN: usize = 32;

/// A persistent node identity: an ed25519 keypair whose public key is the
/// node's address in the mesh.
///
/// The secret key is the 32-byte ed25519 seed; it is serialized raw (not via an
/// iroh re-export, which the blueprint warns is a private pre-release) so the
/// round-trip is exactly `to_secret_bytes` / `from_secret_bytes`.
#[derive(Clone)]
pub struct NodeIdentity {
    signing: SigningKey,
}

impl NodeIdentity {
    /// Generates a fresh random node identity using the platform CSPRNG.
    ///
    /// # Errors
    ///
    /// Returns [`NodeIdentityError::Entropy`] when the platform random source
    /// cannot supply key material.
    pub fn generate() -> Result<Self, NodeIdentityError> {
        let mut seed = [0u8; SECRET_KEY_LEN];
        getrandom::fill(&mut seed).map_err(|err| NodeIdentityError::Entropy(err.to_string()))?;
        Ok(Self::from_secret_bytes(seed))
    }

    /// Reconstructs an identity from its raw 32-byte ed25519 secret seed.
    #[must_use]
    pub fn from_secret_bytes(seed: [u8; SECRET_KEY_LEN]) -> Self {
        Self {
            signing: SigningKey::from_bytes(&seed),
        }
    }

    /// Returns the raw 32-byte ed25519 secret seed for persistence.
    #[must_use]
    pub fn to_secret_bytes(&self) -> [u8; SECRET_KEY_LEN] {
        self.signing.to_bytes()
    }

    /// Returns the ed25519 verifying (public) key.
    #[must_use]
    pub fn verifying_key(&self) -> VerifyingKey {
        self.signing.verifying_key()
    }

    /// Returns this node's own [`PeerId`] (its public key).
    #[must_use]
    pub fn peer_id(&self) -> PeerId {
        PeerId::from_bytes(self.signing.verifying_key().to_bytes())
    }

    /// Loads the identity at `path`, generating and persisting a fresh one with
    /// `0600` permissions if the file does not yet exist.
    ///
    /// The resulting identity is stable across restarts: the same file always
    /// yields the same public key.
    ///
    /// # Errors
    ///
    /// Returns [`NodeIdentityError`] when the key file cannot be read, has the
    /// wrong length, or cannot be created with owner-private permissions.
    pub fn load_or_create(path: impl AsRef<Path>) -> Result<Self, NodeIdentityError> {
        let path = path.as_ref();
        match std::fs::read(path) {
            Ok(bytes) => Self::from_persisted(&bytes),
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                let identity = Self::generate()?;
                identity.persist(path)?;
                Ok(identity)
            }
            Err(err) => Err(NodeIdentityError::Io(err.to_string())),
        }
    }

    /// Returns the default key path `~/.wanix/node.key`.
    ///
    /// # Errors
    ///
    /// Returns [`NodeIdentityError::NoHome`] when no home directory is known.
    pub fn default_key_path() -> Result<PathBuf, NodeIdentityError> {
        let home = home_dir().ok_or(NodeIdentityError::NoHome)?;
        Ok(home.join(".wanix").join("node.key"))
    }

    fn from_persisted(bytes: &[u8]) -> Result<Self, NodeIdentityError> {
        let seed: [u8; SECRET_KEY_LEN] = bytes
            .try_into()
            .map_err(|_| NodeIdentityError::BadLength(bytes.len()))?;
        Ok(Self::from_secret_bytes(seed))
    }

    fn persist(&self, path: &Path) -> Result<(), NodeIdentityError> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)
                .map_err(|err| NodeIdentityError::Io(err.to_string()))?;
        }
        write_owner_private(path, &self.to_secret_bytes())
            .map_err(|err| NodeIdentityError::Io(err.to_string()))
    }
}

impl fmt::Debug for NodeIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Never print secret material.
        f.debug_struct("NodeIdentity")
            .field("peer_id", &self.peer_id())
            .finish()
    }
}

/// Errors produced while creating, loading, or persisting a [`NodeIdentity`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeIdentityError {
    /// The platform random source could not supply key material.
    Entropy(String),
    /// A persisted key file did not contain exactly 32 bytes.
    BadLength(usize),
    /// No home directory was available for the default key path.
    NoHome,
    /// An underlying filesystem operation failed.
    Io(String),
}

impl fmt::Display for NodeIdentityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Entropy(detail) => write!(f, "could not gather entropy: {detail}"),
            Self::BadLength(len) => {
                write!(f, "node key must be {SECRET_KEY_LEN} bytes, found {len}")
            }
            Self::NoHome => f.write_str("no home directory for default node key path"),
            Self::Io(detail) => write!(f, "node key io error: {detail}"),
        }
    }
}

impl std::error::Error for NodeIdentityError {}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
}

#[cfg(unix)]
fn write_owner_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)?;
    file.flush()
}

#[cfg(not(unix))]
fn write_owner_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    std::fs::write(path, bytes)
}

#[cfg(test)]
mod tests;
