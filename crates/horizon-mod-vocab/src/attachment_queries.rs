//! Typed host attachment snapshot queries.
#[cfg(test)]
#[path = "attachment_queries/tests.rs"]
mod tests;
use crate::*;
use horizon_mod_runtime::attachments as runtime;

/// Read queries available on either host side.
pub trait AttachmentQueries: Host {
    /// Capture a SNAPSHOT of membership and value bindings in the current dimension.
    /// Later changes appear only in fresh queries. This is a residency view: an
    /// entity removed but not yet swept may still appear; stale writes fail.
    /// Construction never clones or decodes values; iteration decodes lazily.
    fn query<'key, T: Send + Sync + 'static>(
        &self,
        key: &'key AttachmentType<T>,
    ) -> Result<AttachmentQuery<'key, T>, AttachmentError> {
        self.attachment_query(key.namespace, key.name).query(key)
    }
}
impl<H: Host + ?Sized> AttachmentQueries for H {}

/// Explicit write-through queries, available only on server hosts.
pub trait AttachmentQueriesMut: ServerHost {
    /// Capture a SNAPSHOT, not mutable references into the live store.
    /// Set/remove join the caller's journal; no implicit transaction or Drop commit
    /// occurs. Later changes require a fresh query, and stale writes fail.
    fn query_mut<'key, T: Clone + Send + Sync + 'static>(
        &mut self,
        key: &'key AttachmentType<T>,
    ) -> Result<AttachmentQueryMut<'key, T>, AttachmentError> {
        Ok(AttachmentQueryMut {
            query: self.query(key)?,
            key,
        })
    }
}
impl<H: ServerHost + ?Sized> AttachmentQueriesMut for H {}

// no wire: borrowed runtime attachment rows retain storage lifetimes and write authority.
/// A server write-through snapshot. Values and membership remain captured.
#[non_exhaustive]
pub struct AttachmentQueryMut<'key, T: 'static> {
    query: AttachmentQuery<'key, T>,
    key: &'key AttachmentType<T>,
}
impl<T: Clone + Send + Sync + 'static> AttachmentQueryMut<'_, T> {
    /// Borrow captured values, decoding saved entries only when visited.
    pub fn iter(
        &self,
    ) -> impl ExactSizeIterator<Item = Result<AttachmentRowMut<'_, T>, AttachmentRowError>> {
        self.query
            .iter()
            .map(|row| row.map(|row| AttachmentRowMut { row, key: self.key }))
    }
}

// no wire: borrowed runtime attachment rows retain storage lifetimes and write authority.
/// A borrowed snapshot row with explicit journaled replacement/removal.
#[non_exhaustive]
pub struct AttachmentRowMut<'q, T: 'static> {
    row: runtime::AttachmentRowRef<'q, AttachmentHolder, T>,
    key: &'q AttachmentType<T>,
}
fn write_error(error: AttachmentError) -> AttachmentRowError {
    match error {
        AttachmentError::MissingHolder => AttachmentRowError::StaleHolder,
        AttachmentError::HolderBusy => AttachmentRowError::HolderBusy,
        AttachmentError::UnsupportedHolder(kind) => AttachmentRowError::UnsupportedHolder(kind),
        error => AttachmentRowError::Value(error),
    }
}
impl<T: Clone + Send + Sync + 'static> AttachmentRowMut<'_, T> {
    /// The captured scoped identity.
    pub fn holder(&self) -> AttachmentHolder {
        self.row.holder
    }
    /// The captured value; interior mutability in T is not deep-frozen.
    pub fn value(&self) -> &T {
        self.row.value
    }
    fn holder_for_write(&self, host: &dyn ServerHost) -> Result<Holder, AttachmentRowError> {
        host.attachment_validate_write(self.row.write_token().into())?;
        Ok(match self.row.holder {
            AttachmentHolder::Entity(entity) => Holder::Entity(entity.id()),
            AttachmentHolder::Player(player) => Holder::Player(player.uuid()),
            AttachmentHolder::Chunk(chunk) => Holder::Chunk(chunk.pos()),
            AttachmentHolder::BlockEntity(entity) => Holder::BlockEntity(entity.position),
            AttachmentHolder::Level(_) => Holder::Level,
        })
    }
    /// Replace through the ordinary write path, refusing stale residency first.
    pub fn set(&self, host: &mut dyn ServerHost, value: T) -> Result<(), AttachmentRowError> {
        let holder = self.holder_for_write(host)?;
        self.key
            .set(host.attachments(), holder, value)
            .map_err(write_error)
    }
    /// Remove through the ordinary write path, refusing stale residency first.
    pub fn remove(&self, host: &mut dyn ServerHost) -> Result<Option<T>, AttachmentRowError> {
        let holder = self.holder_for_write(host)?;
        self.key
            .remove(host.attachments(), holder)
            .map_err(write_error)
    }
}
