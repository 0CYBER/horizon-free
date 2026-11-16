//! Identity metadata carried by a real non-list owner. Structural operations use
//! the same journal as joined storage; this object is not a lookup directory.
use super::{Change, Event, Identity, Journal, Owner, Source};
use crate::join_seq::JoinSeq;

#[derive(Debug)]
pub struct Member {
    join: JoinSeq,
    binding: Option<(Journal, Owner, Identity)>,
}
impl Default for Member {
    fn default() -> Self {
        Self {
            join: JoinSeq::next(),
            binding: None,
        }
    }
}
impl Member {
    pub fn join(&self) -> JoinSeq {
        self.join
    }

    pub fn identity(&self) -> Option<Identity> {
        self.binding.as_ref().map(|(_, _, identity)| *identity)
    }

    pub fn location(&self) -> Option<super::table::Location> {
        self.binding
            .as_ref()
            .map(|(_, owner, identity)| super::table::Location {
                owner: *owner,
                join: identity.join,
                source: identity.source,
            })
    }

    /// Replace an actual identity (not a dimension transfer) with a fresh generation.
    pub fn renew(&mut self) {
        self.retire();
        self.join = JoinSeq::next();
    }

    /// Bind or transfer the owner's current identity. Rebinding an epoch or a
    /// temporary extraction restores membership through the ordinary INSTALL.
    pub fn bind(
        &mut self,
        journal: &Journal,
        owner: Owner,
        id: u32,
        source: Source,
        known_uuid: Option<[i32; 4]>,
    ) -> bool {
        if self.binding.as_ref().is_some_and(|(_, _, old)| {
            old.id.get() != id || old.source != source || old.details.known_uuid != known_uuid
        }) {
            self.renew();
        }
        let next = Identity::new(id, self.join)
            .map(|identity| identity.at_source(source).with_known_uuid(known_uuid));
        if let Some((bound, old_owner, old_identity)) = &self.binding
            && bound.same_epoch(journal)
            && *old_owner == owner
            && Some(*old_identity) == next
        {
            return false;
        }
        self.retire();
        if let Some(identity) = next {
            journal.push(Event {
                owner,
                change: Change::Install(identity),
            });
            self.binding = Some((journal.clone(), owner, identity));
            return true;
        }
        false
    }

    /// Rebind an owner whose payload is currently lent outside its slot.
    pub fn rebind(&mut self, journal: &Journal) {
        if let Some((_, owner, identity)) = self.binding.as_ref() {
            let (owner, identity) = (*owner, *identity);
            self.bind(
                journal,
                owner,
                identity.id.get(),
                identity.source,
                identity.details.known_uuid,
            );
        }
    }

    pub fn extract(&self) {
        self.emit(Change::Extract);
    }
    pub fn put_back(&self) {
        self.emit(Change::PutBack);
    }
    pub fn retire(&mut self) {
        self.emit(Change::Remove);
        self.binding = None;
    }
    fn emit(&self, change: fn(Identity) -> Change) {
        if let Some((journal, owner, identity)) = &self.binding {
            journal.push(Event {
                owner: *owner,
                change: change(*identity),
            });
        }
    }
}
impl Drop for Member {
    fn drop(&mut self) {
        self.retire();
    }
}
