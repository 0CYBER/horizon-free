//! The wither's nether star: `WitherBoss.dropCustomDeathLoot` spawns it and
//! then calls `ItemEntity.setExtendedLifetime` (`age = -6000`), so the age
//! discard (`age >= 6000`) takes it at tick 12000 instead of 6000.
use crate::entities::ItemEntity;
use crate::game::Game;
use crate::game::native_machines::tests::server_game;
use crate::items::ItemStack;
use horizon_client::chunk::Chunk;
use horizon_client::mobs::Species;
use horizon_client::motion::PlayerMotion;
use horizon_core::geometry::Vec3;
use horizon_world::nbt::Tag;
use std::sync::Arc;

/// The star the wither leaves starts at `age = -6000`, so it is still there at
/// tick 11999 and discarded at 12000, while an ordinary drop beside it is
/// discarded at 6000.
#[test]
fn a_withers_nether_star_lives_12000_ticks() {
    if std::env::var_os("HORIZON_TEST_RESOURCES").is_none() {
        return; // CI has no prepared resources.
    }
    let mut game = server_game();
    let at = [4.25, 200.0, 4.75];
    // A chunk only ticks its entities once the 5x5 columns around it are in.
    for x in -2..=2 {
        for z in -2..=2 {
            game.world
                .chunks
                .insert((x, z), Arc::new(Chunk::empty(x, z)));
        }
    }
    let stone = game
        .pack
        .assets
        .registry
        .default_state("minecraft:stone")
        .unwrap();
    for x in 0..16 {
        for z in 0..16 {
            game.world.set(x, 199, z, stone).unwrap();
        }
    }
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(8.5, 200.0, 8.5)).unwrap());
    game.update_chunk_tickets();
    assert!(game.entity_ticking_area()(4, 4), "the drops tick");
    let wither = game
        .summon_with_nbt("minecraft:wither", at, "{}")
        .unwrap()
        .expect("the wither is in the world");
    // An ordinary drop beside it, to read the plain 6000-tick lifetime.
    game.summon_with_nbt(
        "minecraft:item",
        [6.25, 200.0, 4.75],
        r#"{Item:{id:"minecraft:stone",count:1}}"#,
    )
    .unwrap();
    let wither_at = game
        .mobs
        .iter()
        .find(|m| m.species() == Species::Wither)
        .map(|m| m.position)
        .expect("the wither mob");
    // `Entity.kill` through `/kill`: the hit runs LivingEntity.die now.
    game.kill_target(wither);
    let star = |game: &Game<'_>| {
        game.dropped
            .iter()
            .find(|i| &*i.stack.item == "minecraft:nether_star")
            .map(|i| (i.age, i.position))
    };
    let (age, position) = star(&game).expect("the wither drops its nether star");
    assert_eq!(age, -6000, "ItemEntity.setExtendedLifetime");
    assert_eq!(
        [position.x, position.y, position.z],
        [wither_at.x, wither_at.y + 0.5, wither_at.z],
        "spawnAtLocation at the death drop's position"
    );
    let ordinary = |game: &Game<'_>| {
        game.dropped
            .iter()
            .any(|i| &*i.stack.item == "minecraft:stone")
    };
    assert!(ordinary(&game), "the ordinary drop is in the world");
    // Both drops joined after the last tick's entity pass: they tick from now.
    game.ticked_entities_through = game.next_entity;
    let tick = |game: &mut Game<'_>, n: u32| {
        for _ in 0..n {
            game.tick_items();
        }
    };
    // At 5999 ticks both are still there; the ordinary one is discarded on its 6000th.
    tick(&mut game, 5999);
    assert_eq!(star(&game).map(|s| s.0), Some(-1));
    assert!(ordinary(&game));
    tick(&mut game, 1);
    assert!(!ordinary(&game), "an ordinary drop is discarded at 6000");
    assert_eq!(star(&game).map(|s| s.0), Some(0), "the star is still there");
    tick(&mut game, 5999);
    assert!(star(&game).is_some(), "the star survives to tick 11999");
    tick(&mut game, 1);
    assert!(star(&game).is_none(), "the star is discarded at tick 12000");
}

/// `ItemEntity.addAdditionalSaveData` saves `Age` as a short, so the star's
/// negative age is still there after a save and load.
#[test]
fn a_stars_negative_age_survives_item_nbt() {
    let mut star = ItemEntity::new(
        7,
        ItemStack::new("minecraft:nether_star", 1),
        Vec3::new(4.5, 200.0, 4.5),
        Vec3::ZERO,
        0.0,
    );
    star.age = -6000;
    let tag = crate::entity_nbt::item_entity_to_nbt(&star, [1, 2, 3, 4], None);
    assert_eq!(tag.get("Age"), Some(&Tag::Short(-6000)));
    let loaded = crate::entity_nbt::item_entity_from_nbt(&tag, 7, 0.0).unwrap();
    assert_eq!(loaded.age, -6000);
    assert_eq!(&*loaded.stack.item, "minecraft:nether_star");
}
