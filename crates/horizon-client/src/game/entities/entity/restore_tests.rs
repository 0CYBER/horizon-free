use crate::game::native_entity_refs::tests::with_game;
use horizon_client::join_seq::Joined;
use horizon_mod_runtime::attachments::{
    AttachmentType, Attachments, HasAttachmentLifetime, Holder, index::AttachmentIndex,
};
use std::sync::{Arc, Mutex};

// Use the real NBT spawn loader, including its id allocation and join hooks,
// while every list is held by the production take/restore boundary.
macro_rules! restore_cases {
    ($($name:ident: $kind:literal, $nbt:literal, $($field:ident).+;)*) => {$ (
        #[test]
        #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
        fn $name() {
            with_game(|game| {
                game.summon_with_nbt($kind, [0.5, 70.0, 0.5], $nbt).unwrap();
                game.lan_assign_ids();
                let prefix: Vec<_> = game$(.$field)+.iter().map(|e| (e.id, e.join_seq())).collect();
                assert_eq!(prefix.len(), 1);
                let taken = game.take_entities();
                game.summon_with_nbt($kind, [4.5, 70.0, 0.5], $nbt).unwrap();
                game.lan_assign_ids();
                let suffix: Vec<_> = game$(.$field)+.iter().map(|e| (e.id, e.join_seq())).collect();
                assert_eq!(suffix.len(), 1);
                assert!(prefix[0].1 < suffix[0].1);
                assert_ne!(prefix[0].0, suffix[0].0);
                game.restore_entities(taken);
                let restored: Vec<_> = game$(.$field)+.iter().map(|e| (e.id, e.join_seq())).collect();
                assert_eq!(restored, [prefix, suffix].concat());
            });
        }
    )*};
}

restore_cases! {
    restores_mob_spawns: "minecraft:pig", "{}", mobs;
    restores_item_spawns: "minecraft:item", "{Item:{id:\"minecraft:stone\",count:1}}", dropped;
    restores_boat_spawns: "minecraft:oak_boat", "{}", boats;
    restores_cart_spawns: "minecraft:minecart", "{}", carts;
    restores_stand_spawns: "minecraft:armor_stand", "{}", armor_stands;
    restores_tnt_spawns: "minecraft:tnt", "{fuse:80s}", tnts;
    restores_falling_spawns: "minecraft:falling_block", "{BlockState:{Name:\"minecraft:sand\"},Time:1}", falling;
    restores_orb_spawns: "minecraft:experience_orb", "{Value:3s}", orbs;
    restores_projectile_spawns: "minecraft:arrow", "{}", creatures.projectiles;
    restores_rocket_spawns: "minecraft:firework_rocket", "{LifeTime:20}", rockets;
    restores_trident_spawns: "minecraft:trident", "{}", tridents;
}

static VALUE: AttachmentType<i32> = AttachmentType {
    namespace: "test",
    name: "restore_lifetime",
    default: || 0,
    codec: None,
    copy_on_death: false,
    sync: false,
};

#[test]
#[ignore = "daily"]
fn restoring_keeps_both_owners_bound_and_old_batches_immutable() {
    with_game(|game| {
        let index = Arc::new(Mutex::new(AttachmentIndex::default()));
        let key = ("test".into(), "restore_lifetime".into());
        let mut values = Attachments::default();
        game.summon_with_nbt("minecraft:oak_boat", [0.5, 70.0, 0.5], "{}")
            .unwrap();
        let original = game.boats[0].id;
        let holder = game.entity(original).unwrap();
        VALUE
            .set(&mut values, Holder::Entity(original), 10)
            .unwrap();
        let registration = game.boats.update(0, |boat| {
            boat.attachment_lifetime().bind(&mut values, &index, holder);
            boat.attachment_lifetime().registration().unwrap()
        });
        let before = index.lock().unwrap().snapshot(&key);
        let taken = game.take_entities();
        assert_eq!(index.lock().unwrap().snapshot(&key).len(), 1);
        game.summon_with_nbt("minecraft:oak_boat", [4.5, 70.0, 0.5], "{}")
            .unwrap();
        let spawned = game.boats[0].id;
        let holder = game.entity(spawned).unwrap();
        let mut spawned_values = Attachments::default();
        VALUE
            .set(&mut spawned_values, Holder::Entity(spawned), 20)
            .unwrap();
        let spawn_registration = game.boats.update(0, |boat| {
            boat.attachment_lifetime()
                .bind(&mut spawned_values, &index, holder);
            boat.attachment_lifetime().registration().unwrap()
        });
        game.restore_entities(taken);
        assert_eq!(
            game.boats.iter().map(|b| b.id).collect::<Vec<_>>(),
            [original, spawned]
        );
        assert_eq!(index.lock().unwrap().snapshot(&key).len(), 2);
        for (i, expected) in [(0, registration), (1, spawn_registration)] {
            let actual = game
                .boats
                .update(i, |boat| boat.attachment_lifetime().registration().unwrap());
            assert!(expected.same_registration(&actual));
        }
        assert_eq!(VALUE.get(&mut values, Holder::Entity(original)), Ok(10));
        assert_eq!(
            VALUE.get(&mut spawned_values, Holder::Entity(spawned)),
            Ok(20)
        );
        VALUE
            .set(&mut values, Holder::Entity(original), 30)
            .unwrap();
        let after = index.lock().unwrap().snapshot(&key);
        assert_eq!(after.len(), 2);
        drop(game.boats.remove(0));
        let surviving = index.lock().unwrap().snapshot(&key);
        assert_eq!(surviving.len(), 1);
        assert_eq!(surviving[0].holder, holder);
        assert_eq!(before.len(), 1);
        assert_eq!(before[0].entry.read(&VALUE), Ok(&10));
        assert_eq!(after.len(), 2);
        assert_eq!(
            VALUE.get(&mut spawned_values, Holder::Entity(spawned)),
            Ok(20)
        );
    });
}
