//! A packet body on its way to a client: its own bytes, or bytes several clients share.

use std::sync::Arc;

/// A packet body (VarInt id and payload) on its way to one client. Entity tracking sends the
/// same bodies to every client tracking an entity: those hold one shared copy, where a
/// body sent to one client keeps its own bytes without copying them again. Either goes on
/// the wire the same way.
#[derive(Clone)]
pub enum Body {
    Owned(Vec<u8>),
    Shared(Arc<[u8]>),
}

impl Body {
    /// Bytes several clients will be sent.
    pub fn shared(bytes: Vec<u8>) -> Self {
        Self::Shared(bytes.into())
    }
    /// The bytes as a vector of their own (copied when shared).
    pub fn into_vec(self) -> Vec<u8> {
        match self {
            Self::Owned(bytes) => bytes,
            Self::Shared(bytes) => bytes.to_vec(),
        }
    }
}

impl std::ops::Deref for Body {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        match self {
            Self::Owned(bytes) => bytes,
            Self::Shared(bytes) => bytes,
        }
    }
}

impl From<Vec<u8>> for Body {
    fn from(bytes: Vec<u8>) -> Self {
        Self::Owned(bytes)
    }
}

/// Bodies are equal when their bytes are, shared or not.
impl PartialEq for Body {
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}
impl Eq for Body {}
impl PartialEq<Vec<u8>> for Body {
    fn eq(&self, other: &Vec<u8>) -> bool {
        **self == other[..]
    }
}

impl std::fmt::Debug for Body {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&**self, f)
    }
}
