use super::*;
use horizon_core::geometry::Vec3;
use horizon_mod_runtime::attachments::{
    AttachmentType, Attachments, HasAttachmentLifetime, Holder, index::AttachmentIndex,
};
use std::sync::{Arc, Mutex};

static VALUE: AttachmentType<i32> = AttachmentType {
    namespace: "test",
    name: "golem_lifetime",
    default: || 0,
    codec: None,
    copy_on_death: false,
    sync: false,
};

#[test]
#[ignore = "daily"]
fn statue_conversion_keeps_the_owner_until_physical_discard() {
    super::super::native_entity_refs::tests::with_game(|game| {
        game.world
            .chunks
            .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
        let kind = game
            .res
            .mob_types()
            .get("minecraft:copper_golem")
            .unwrap()
            .clone();
        game.mobs
            .push(Mob::new(71, kind, Vec3::new(0.5, 70.0, 0.5), 0.0));
        let index = Arc::new(Mutex::new(AttachmentIndex::default()));
        let holder = mod_api::EntityRef::new(game.native_lifecycle.epoch().unwrap(), 71);
        let mut values = Attachments::default();
        VALUE.set(&mut values, Holder::Entity(71), 10).unwrap();
        let original = game.mobs.update(0, |mob| {
            mob.attachment_lifetime().bind(&mut values, &index, holder);
            mob.attachment_lifetime().registration().unwrap()
        });
        let key = ("test".into(), "golem_lifetime".into());
        let before = index.lock().unwrap().snapshot(&key);
        assert_eq!(before.len(), 1);
        game.golem_become_statue(71);
        assert!(game.mobs[0].removed);
        let current = game
            .mobs
            .update(0, |mob| mob.attachment_lifetime().registration().unwrap());
        assert!(original.same_registration(&current));
        assert_eq!(index.lock().unwrap().snapshot(&key).len(), 1);
        VALUE.set(&mut values, Holder::Entity(71), 20).unwrap();
        let after = index.lock().unwrap().snapshot(&key);
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].entry.read(&VALUE), Ok(&20));
        game.mobs.retain(|mob| !mob.removed);
        assert!(index.lock().unwrap().snapshot(&key).is_empty());
        assert_eq!(before[0].entry.read(&VALUE), Ok(&10));
        assert_eq!(after[0].entry.read(&VALUE), Ok(&20));
    });
}
