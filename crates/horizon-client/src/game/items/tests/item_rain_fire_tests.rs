//! Rain puts a burning dropped item out, as `Entity.applyEffectsFromBlocks`
//! does, and the hiss `Entity.playEntityOnFireExtinguishedSound` plays follows.
use super::super::native_machines::tests::server_game;
use super::*;
use horizon_core::random::{LegacyRandom, RandomSource};
use std::sync::Arc;

/// A server's game under rain over plains. `Level.isRainingAt` needs rain at
/// full level, sky light 15 (an empty section holds it), nothing blocking the
/// sky and a biome whose precipitation is rain, so the worldgen features the
/// game installs when a world opens must be there too.
fn raining_game() -> Game<'static> {
    let mut game = server_game();
    game.install_live_features();
    game.world.weather.raining = true;
    game.world.weather.rain_level = (1.0, 1.0);
    let plains = u32::from(game.res.biomes().id("minecraft:plains"));
    let chunk = Arc::make_mut(game.world.chunks.get_mut(&(0, 0)).unwrap());
    for section in &mut chunk.sections {
        let section = Arc::make_mut(section);
        for i in 0..64 {
            section.biomes.set(i, plains);
        }
    }
    game
}

fn hisses(game: &Game<'_>) -> Vec<(String, [f64; 3], f32, f32)> {
    game.audio
        .capture
        .iter()
        .flatten()
        .map(|s| (s.event.clone(), s.position, s.volume, s.pitch))
        .collect()
}

/// `Entity.applyEffectsFromBlocks` clears the fire of an item in rain and plays
/// `playEntityOnFireExtinguishedSound`: `entity.generic.extinguish_fire` at the
/// item, volume 0.7, its pitch from two draws of the item's own random. A
/// fire-immune stack lost its fire in `Entity.baseTick`, before the `wasOnFire`
/// the hiss compares, so the rain is silent for it.
#[test]
#[ignore = "daily"]
fn rain_puts_a_burning_item_out_with_the_hiss() {
    let mut game = raining_game();
    game.spawn_item(
        ItemStack::new("minecraft:oak_planks", 1),
        [8.5, 100.0, 8.5],
        [0.0; 3],
        0,
    );
    game.spawn_item(
        ItemStack::new("minecraft:netherite_ingot", 1),
        [10.5, 100.0, 10.5],
        [0.0; 3],
        0,
    );
    game.ticked_entities_through = game.next_entity;
    for item in &mut game.dropped {
        item.fire_ticks = 400;
        item.age = 10;
    }
    // The hiss takes its pitch from the item's own random, in vanilla's order.
    *game.dropped[0].random = LegacyRandom::new(11);
    let mut random = LegacyRandom::new(11);
    let pitch = 1.6 + (random.next_float() - random.next_float()) * 0.4;
    game.audio.capture = Some(Vec::new());
    game.tick_items();
    assert_eq!(game.dropped.len(), 2, "the rain does not destroy them");
    let fires: Vec<i32> = game.dropped.iter().map(|item| item.fire_ticks).collect();
    assert_eq!(fires, [0, 0], "one tick of rain puts them out");
    assert_eq!(
        game.dropped[0].age, 11,
        "and the burning one ticks normally"
    );
    let p = game.dropped[0].position;
    assert_eq!(
        hisses(&game),
        [(
            "minecraft:entity.generic.extinguish_fire".to_owned(),
            [p.x, p.y, p.z],
            0.7,
            pitch
        )],
        "one hiss, for the stack that burned, with its random's pitch"
    );
}

/// `applyEffectsFromBlocks` asks `isInRain` after `checkInsideBlocks`, so the
/// rain undoes a fire block's `BaseFireBlock.fireIgnite` in the same tick, and
/// `wasOnFire` (taken before it) is false: no hiss.
#[test]
#[ignore = "daily"]
fn rain_puts_out_what_a_fire_block_ignites_in_the_same_tick() {
    let mut game = raining_game();
    let stone = game
        .pack
        .assets
        .registry
        .default_state("minecraft:stone")
        .unwrap();
    let fire = game
        .pack
        .assets
        .registry
        .default_state("minecraft:fire")
        .unwrap();
    game.world.set(8, 99, 8, stone).unwrap();
    game.world.set(8, 100, 8, fire).unwrap();
    game.spawn_item(
        ItemStack::new("minecraft:oak_planks", 1),
        [8.5, 100.0, 8.5],
        [0.0; 3],
        0,
    );
    game.ticked_entities_through = game.next_entity;
    game.audio.capture = Some(Vec::new());
    game.tick_items();
    assert_eq!(
        game.dropped[0].fire_ticks, 0,
        "the fire block's 160 ticks are put out at once"
    );
    assert!(
        hisses(&game).is_empty(),
        "and the item was not burning before the block ignited it"
    );
}
