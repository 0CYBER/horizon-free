//! Block entities a moving piston carries (arismoko/horizon#3184): taken out of the world with
//! their attachments and without removal side effects, held by the moving
//! piston's position, and installed where their block lands as a new holder.
//! Each has one owner at a time: it is moved, never cloned, so what it holds
//! can't be duplicated.
use super::{BlockEntities, BlockEntity, Pos, identity};
use horizon_mod_runtime::attachments::Attachments;

/// A block entity out of the world while a piston moves its block.
pub struct Carried {
    pub entity: BlockEntity,
    /// Position-owned saved fields and adapter state, owned by the carried instance.
    pub position_data: horizon_core::nbt::Compound,
    /// Its attachment map, when it had one.
    pub attachments: Option<Attachments>,
}

impl BlockEntities {
    /// Takes the block entity at `from` out of the world for the moving piston
    /// at `to`. Its slot is freed and the attachment index sees the holder
    /// leave, but it is not removed: no side effects run, so nothing it holds
    /// drops. False when there is none, or it is out for its own tick.
    pub fn carry(&mut self, from: Pos, to: Pos) -> bool {
        if !self.map.contains_key(&from) {
            return false;
        }
        let attachments = self.attachments.remove(&from);
        if attachments.is_some()
            && let Some(observer) = &self.attachment_observer
        {
            observer.unload(identity(from, &self.generations));
        }
        let Some(entity) = self.remove(&from) else {
            return false;
        };
        self.carried.insert(
            to,
            Carried {
                entity,
                position_data: Default::default(),
                attachments,
            },
        );
        true
    }
    /// Installs the block entity carried to `at` there as a new holder: a new
    /// ticker slot at the end of the list, and its attachments bound again.
    /// False when none was carried there.
    pub fn land(&mut self, at: Pos) -> bool {
        let Some(Carried {
            entity,
            attachments,
            ..
        }) = self.carried.remove(&at)
        else {
            return false;
        };
        self.remove(&at);
        self.insert(at, entity);
        if let Some(attachments) = attachments {
            self.restore_attachments(at, attachments);
        }
        true
    }
    /// The block entity carried to `at`, for its moving piston's save.
    pub fn carried(&self, at: &Pos) -> Option<&Carried> {
        self.carried.get(at)
    }
    /// An in-flight tick owns the payload: a piston must refuse rather than replace its lease.
    pub fn is_taken(&self, at: &Pos) -> bool {
        self.out.contains_key(at)
    }
    /// The owning adapter updates position-owned state before a save or landing.
    pub fn carried_mut(&mut self, at: &Pos) -> Option<&mut Carried> {
        self.carried.get_mut(at)
    }
    /// Holds `carried` for the moving piston at `at`, loaded with it.
    pub fn hold_carried(&mut self, at: Pos, carried: Carried) {
        self.carried.insert(at, carried);
    }
    /// Drops the block entity carried to `at` without installing it: a
    /// client's copy once the server's block took the moving piston's place.
    pub fn discard_carried(&mut self, at: &Pos) -> bool {
        self.carried.remove(at).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::super::{BlockEntityIdentity, Ticking};
    use super::*;
    use horizon_mod_runtime::attachments::{
        AttachmentType, Holder, index::AttachmentIndex, observer::IndexedAttachmentObserver,
    };
    use std::sync::{Arc, Mutex};

    static VALUE: AttachmentType<i32> = AttachmentType {
        namespace: "carry_test",
        name: "value",
        default: || 0,
        codec: None,
        copy_on_death: false,
        sync: false,
    };

    #[test]
    fn a_carried_block_entity_leaves_its_holder_and_lands_as_a_new_one() {
        let index = Arc::new(Mutex::new(AttachmentIndex::<BlockEntityIdentity>::default()));
        let mut entities = BlockEntities::default();
        entities.observe_attachments(Arc::new(IndexedAttachmentObserver::new(&index, Some)));
        entities.set_states(vec![Ticking::Yes]);
        let (from, to) = ((1, 64, 1), (2, 64, 1));
        entities.insert(from, BlockEntity::Chest(vec![None, None, None]));
        VALUE
            .set(entities.attachments_mut(from).unwrap(), Holder::Level, 7)
            .unwrap();
        let rows = || {
            index
                .lock()
                .unwrap()
                .snapshot(&("carry_test".into(), "value".into()))
                .iter()
                .map(|row| (row.holder.position, *row.entry.read(&VALUE).unwrap()))
                .collect::<Vec<_>>()
        };
        assert_eq!(entities.tickers(|_| 0).len(), 1);

        assert!(entities.carry(from, to));
        assert!(!entities.contains_key(&from) && !entities.contains_key(&to));
        assert!(entities.attachments(from).is_none());
        assert!(rows().is_empty());
        assert!(entities.tickers(|_| 0).is_empty());

        assert!(entities.land(to));
        assert!(matches!(entities.get(&to), Some(BlockEntity::Chest(slots)) if slots.len() == 3));
        assert_eq!(rows(), [([2, 64, 1], 7)]);
        assert_eq!(
            entities
                .tickers(|_| 0)
                .iter()
                .map(|(p, _)| *p)
                .collect::<Vec<_>>(),
            [to]
        );
        assert!(!entities.land(to));
    }

    #[test]
    fn unloading_its_chunk_drops_a_carried_block_entity() {
        let mut entities = BlockEntities::default();
        entities.insert((1, 64, 1), BlockEntity::Chest(vec![]));
        assert!(entities.carry((1, 64, 1), (17, 64, 1)));
        entities.unload(|p| p.0 >> 4 == 1);
        assert!(entities.carried(&(17, 64, 1)).is_none());
        assert!(!entities.land((17, 64, 1)));
    }
}
