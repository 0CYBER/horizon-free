//! Owning storage for a resident holder, distinct from detached map snapshots.

use super::{
    AttachmentAccess, AttachmentError, AttachmentHost, AttachmentLifetime, AttachmentRegistration,
    Attachments, Holder, index::AttachmentIndex,
};
use std::{
    hash::Hash,
    ops::{Deref, DerefMut},
    sync::{Arc, Mutex},
};

/// Canonical storage whose lifetime is independent of borrowed map contents.
///
/// Moves preserve publication. Clones contain detached data only, so saving or
/// journaling a clone never owns the original holder's residency.
#[doc(hidden)]
#[derive(Clone, Debug, Default)]
pub struct ResidentAttachments {
    contents: Attachments,
    lifetime: AttachmentLifetime,
}

impl ResidentAttachments {
    /// Bind the owning storage, not a borrowed data map.
    pub fn bind_index<H: Copy + Eq + Hash + Send + Sync + 'static>(
        &mut self,
        index: &Arc<Mutex<AttachmentIndex<H>>>,
        holder: H,
    ) -> AttachmentRegistration {
        let binding = self.contents.bind_index(index, holder);
        self.lifetime = AttachmentLifetime::new(binding.clone());
        binding
    }

    /// Copy the current registration without owning its lifetime.
    pub fn registration(&self) -> Option<AttachmentRegistration> {
        self.lifetime.registration()
    }

    /// Retire before a removed owner is returned or retained across callbacks.
    pub fn retire(&mut self) {
        self.lifetime = AttachmentLifetime::default();
    }
}

impl super::HasAttachmentLifetime for ResidentAttachments {
    fn attachment_lifetime(&mut self) -> &mut AttachmentLifetime {
        &mut self.lifetime
    }
}

impl Drop for ResidentAttachments {
    fn drop(&mut self) {
        self.retire();
    }
}

impl Deref for ResidentAttachments {
    type Target = Attachments;
    fn deref(&self) -> &Self::Target {
        &self.contents
    }
}

impl DerefMut for ResidentAttachments {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.contents
    }
}

impl AttachmentHost for ResidentAttachments {
    fn attachments(
        &mut self,
        _: Holder,
        _: AttachmentAccess,
    ) -> Result<&mut Attachments, AttachmentError> {
        Ok(&mut self.contents)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attachments::{AttachmentType, HasAttachmentLifetime, index::AttachmentIndex};

    #[test]
    fn real_wrapper_lifetime_owns_external_store_registration_through_moves_and_clones() {
        static VALUE: AttachmentType<i32> = AttachmentType {
            namespace: "test",
            name: "resident_lifetime",
            default: || 0,
            codec: None,
            copy_on_death: false,
            sync: false,
        };
        let index = Arc::new(Mutex::new(AttachmentIndex::<u32>::default()));
        let mut map = Attachments::default();
        VALUE.set(&mut map, Holder::Level, 7).unwrap();
        let mut owner = ResidentAttachments::default();
        owner.attachment_lifetime().bind(&mut map, &index, 19);
        let key = ("test".into(), "resident_lifetime".into());
        let rows = index.lock().unwrap().snapshot(&key);
        assert_eq!(rows[0].holder, 19);
        assert_eq!(rows[0].entry.read(&VALUE), Ok(&7));
        drop(owner.clone());
        let moved = vec![owner];
        assert_eq!(index.lock().unwrap().snapshot(&key).len(), 1);
        drop(moved);
        assert!(index.lock().unwrap().snapshot(&key).is_empty());
        VALUE.set(&mut map, Holder::Level, 23).unwrap();
        assert!(index.lock().unwrap().snapshot(&key).is_empty());
        assert_eq!(VALUE.get(&mut map, Holder::Level), Ok(23));
        assert_eq!(rows[0].entry.read(&VALUE), Ok(&7));
    }
}
