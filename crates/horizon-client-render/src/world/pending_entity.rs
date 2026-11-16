//! Pending entity NBT and the runtime context that must not be stored inside it.
use horizon_core::nbt::Tag;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntityLoadReason {
    Ordinary,
    TeleportedTnt,
}

#[derive(Clone, Debug)]
pub struct PendingEntity {
    tag: Tag,
    reason: EntityLoadReason,
}

impl From<Tag> for PendingEntity {
    fn from(tag: Tag) -> Self {
        Self {
            tag,
            reason: EntityLoadReason::Ordinary,
        }
    }
}

impl PendingEntity {
    /// Only an in-memory TNT transfer supplies this context, never saved NBT.
    pub fn after_tnt_portal(tag: Tag) -> Self {
        Self {
            tag,
            reason: EntityLoadReason::TeleportedTnt,
        }
    }

    pub fn into_parts(self) -> (Tag, EntityLoadReason) {
        (self.tag, self.reason)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_nbt_cannot_request_transient_portal_state() {
        for marker in [0, 1] {
            let tag = Tag::compound([("horizon:tnt_used_portal", Tag::Byte(marker))]);
            let (loaded, reason) = PendingEntity::from(tag.clone()).into_parts();
            assert_eq!(loaded, tag);
            assert_eq!(reason, EntityLoadReason::Ordinary);
        }
    }

    #[test]
    fn queue_swaps_and_appends_keep_each_entitys_context_and_order() {
        let ordinary = Tag::compound([("id", Tag::String("ordinary".into()))]);
        let transfer = Tag::compound([("id", Tag::String("transfer".into()))]);
        let mut batches = vec![(
            (1, 2),
            vec![
                ordinary.clone().into(),
                PendingEntity::after_tnt_portal(transfer.clone()),
            ],
        )];
        let mut waiting = std::mem::take(&mut batches);
        batches.push(((3, 4), vec![ordinary.clone().into()]));
        waiting.append(&mut batches);
        assert!(batches.is_empty());
        let mut batches = waiting.into_iter();
        let (key, entities) = batches.next().unwrap();
        assert_eq!(key, (1, 2));
        let mut entities = entities.into_iter();
        assert_eq!(
            entities.next().unwrap().into_parts(),
            (ordinary.clone(), EntityLoadReason::Ordinary)
        );
        assert_eq!(
            entities.next().unwrap().into_parts(),
            (transfer, EntityLoadReason::TeleportedTnt)
        );
        assert!(entities.next().is_none());
        let (key, entities) = batches.next().unwrap();
        assert_eq!(key, (3, 4));
        assert_eq!(
            entities.into_iter().next().unwrap().into_parts(),
            (ordinary, EntityLoadReason::Ordinary)
        );
        assert!(batches.next().is_none());
    }
}
