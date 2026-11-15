//! `CactusBlock.entityInside` for a boat, a minecart and an experience orb,
//! and `VehicleEntity.hurtServer`'s `entity_damage` (repro of the issue).
use super::super::native_machines::tests::server_game;
use super::*;
use horizon_client::chunk::Chunk;
use horizon_core::position::ChunkPos;
use std::sync::Arc;

type Posted = (&'static str, [f64; 3], Option<Who>);

fn capture() {
    crate::game::sculk::TEST_POSTED_EVENTS.with(|e| *e.borrow_mut() = Some(Vec::new()));
}

fn damage_events() -> Vec<Posted> {
    let events =
        crate::game::sculk::TEST_POSTED_EVENTS.with(|e| e.borrow_mut().take().unwrap_or_default());
    events
        .into_iter()
        .filter(|e| e.0 == "minecraft:entity_damage")
        .collect()
}

/// A forced-loaded chunk with a stone floor at y 99 and a cactus on sand at
/// each of `cacti` (x, z) at y 100.
fn scene(cacti: &[(i32, i32)]) -> Game<'static> {
    let mut game = server_game();
    game.build_registries();
    game.set_allow_commands(true);
    let registry = &game.pack.assets.registry;
    let stone = registry.default_state("minecraft:stone").unwrap();
    let mut chunk = Chunk::empty(0, 0);
    for x in 0..16 {
        for z in 0..16 {
            chunk.set(x, 99, z, stone.0);
        }
    }
    game.world.chunks.insert((0, 0), Arc::new(chunk));
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    game.load_ticking_neighbourhood(0, 0);
    assert!(game.run_command("forceload add 0 0"));
    for &(x, z) in cacti {
        assert!(game.run_command(&format!("setblock {x} 99 {z} minecraft:sand")));
        assert!(game.run_command(&format!("setblock {x} 100 {z} minecraft:cactus")));
    }
    game
}

/// `AbstractBoat.tick` runs `applyEffectsFromBlocks` twice after its move, so
/// a boat in a cactus takes two hits a tick (`VehicleEntity.hurtServer`: 10
/// damage each, after the tick's wear of 1): 20, then 39, and the first hit
/// of the third tick passes 40 and breaks it. Each hit is heard at the boat.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_boat_in_a_cactus_takes_two_hits_a_tick_and_each_is_heard() {
    let mut game = scene(&[(4, 4)]);
    assert!(game.run_command("summon minecraft:oak_boat 4.5 100 4.5"));
    capture();
    game.tick_world(true).unwrap();
    assert_eq!(game.boats.len(), 1);
    let b = &game.boats[0];
    assert_eq!((b.damage, b.hurt_time), (20.0, 10));
    let at = [b.position.x, b.position.y, b.position.z];
    let expected: Vec<Posted> = vec![("minecraft:entity_damage", at, None); 2];
    assert_eq!(damage_events(), expected);
    game.tick_world(true).unwrap();
    assert_eq!(game.boats[0].damage, 39.0);
    game.tick_world(true).unwrap();
    assert!(
        game.boats.is_empty(),
        "the third tick's first hit breaks it"
    );
}

/// A minecart's behaviour runs `applyEffectsFromBlocks` too, so a cactus
/// wears it down and breaks it, and an orb's `ExperienceOrb.tick` loses a
/// point of its 5 health a tick to one.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_cactus_hurts_a_minecart_and_an_experience_orb() {
    let mut game = scene(&[(10, 4), (4, 10)]);
    assert!(game.run_command("summon minecraft:minecart 10.5 100 4.5"));
    assert!(
        game.run_command("summon minecraft:experience_orb 4.5 100.25 10.5 {Value:1s,Health:5s}")
    );
    capture();
    game.tick_world(true).unwrap();
    let c = &game.carts[0];
    assert!(c.damage >= 10.0 && c.hurt_time == 10, "{}", c.damage);
    let at = [c.position.x, c.position.y, c.position.z];
    let events = damage_events();
    assert!(
        !events.is_empty()
            && events
                .iter()
                .all(|e| *e == ("minecraft:entity_damage", at, None))
    );
    assert_eq!(game.orbs[0].health, 4);
    for _ in 0..10 {
        game.tick_world(true).unwrap();
    }
    assert!(game.carts.is_empty(), "the cactus breaks the cart");
}
