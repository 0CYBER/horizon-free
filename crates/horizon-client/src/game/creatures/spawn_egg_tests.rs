//! Spawn eggs: the egg's components on its child, the block surface it spawns
//! on, and the zombie offspring's `CanPickUpLoot` roll.
use super::*;
use crate::game::Hit;
use horizon_client::mobs::Species;
use horizon_core::nbt::Tag;
use horizon_core::random::{EntityRandom, LegacyRandom, RandomSource};
use horizon_world::food::Difficulty;

fn game() -> Option<Game<'static>> {
    std::env::var_os("HORIZON_TEST_RESOURCES")?;
    let mut game = crate::game::native_machines::tests::server_game();
    game.build_registries();
    game.set_allow_commands(true);
    game.player = None;
    Some(game)
}

fn spawn(game: &mut Game<'_>, kind: &str) -> u32 {
    game.spawn_mob_spawned(
        kind,
        [4.5, 100.0, 4.5],
        SpawnSite::Plain,
    )
    .unwrap()
}

#[test]
fn an_egg_child_takes_the_stacks_name_and_data() {
    let Some(mut game) = game() else { return };
    let parent = spawn(&mut game, "minecraft:pig");
    let index = game.mobs.iter().position(|m| m.id == parent).unwrap();
    let mut egg = ItemStack::new("minecraft:pig_spawn_egg", 1);
    egg.data_mut().custom_name = Some(Tag::String("Porky".into()));
    egg.data_mut().set_other(
        "minecraft:custom_data",
        Some(Tag::compound([("audit", Tag::Int(7))])),
    );
    *game.held_mut() = Some(egg);
    assert!(game.spawn_egg_offspring(index));
    let child = game
        .mobs
        .iter()
        .position(|m| m.id != parent && m.species() == Species::Pig)
        .unwrap();
    assert_eq!(game.mobs[child].custom_name.as_deref(), Some("Porky"));
    let saved = game.mob_tree_nbt(child);
    assert_eq!(
        saved.get("data"),
        Some(&Tag::compound([("audit", Tag::Int(7))]))
    );
}

#[test]
fn a_block_egg_spawns_on_the_block_surface() {
    let Some(mut game) = game() else { return };
    let slab = game
        .pack
        .assets
        .registry
        .default_state("minecraft:stone_slab")
        .unwrap();
    game.world.set(4, 100, 4, slab).unwrap();
    let mut stack = ItemStack::new("minecraft:pig_spawn_egg", 1);
    stack.data_mut().set_other(
        "minecraft:custom_data",
        Some(Tag::compound([("audit", Tag::Int(9))])),
    );
    *game.held_mut() = Some(stack.clone());
    game.tick_hit.set(Some(Some(Hit {
        pos: [4, 100, 4],
        face: [0, 1, 0],
        point: [4.5, 100.5, 4.5],
        inside: false,
    })));
    assert!(game.use_spawn_egg(&stack).unwrap());
    let pig = game
        .mobs
        .iter()
        .position(|m| m.species() == Species::Pig)
        .unwrap();
    assert_eq!(game.mobs[pig].position.y, 100.5);
    // `EntityType.create`'s `PostSpawnProcessor`: the egg's components too.
    assert_eq!(
        game.mob_tree_nbt(pig).get("data"),
        Some(&Tag::compound([("audit", Tag::Int(9))]))
    );
    // The dragon has no `mob_types` entry: `getYOffset` still uses its 16 by 8
    // box, so it lands on the slab's surface as well.
    let dragon = ItemStack::new("minecraft:ender_dragon_spawn_egg", 1);
    *game.held_mut() = Some(dragon.clone());
    game.tick_hit.set(Some(Some(Hit {
        pos: [4, 100, 4],
        face: [0, 1, 0],
        point: [4.5, 100.5, 4.5],
        inside: false,
    })));
    assert!(game.use_spawn_egg(&dragon).unwrap());
    assert_eq!(game.dragon.as_ref().unwrap().position.y, 100.5);
}

#[test]
fn an_egg_born_zombie_rolls_can_pick_up_loot_from_the_parent() {
    let Some(mut game) = game() else { return };
    game.world.health_rules.difficulty = Difficulty::Hard;
    let parent = spawn(&mut game, "minecraft:zombie");
    let index = game.mobs.iter().position(|m| m.id == parent).unwrap();
    let at = [
        game.mobs[index].position.x,
        game.mobs[index].position.y,
        game.mobs[index].position.z,
    ];
    let special = game.local_difficulty(at).special();
    assert!(special > 0.0, "hard difficulty has a special multiplier");
    let seed = (0..)
        .find(|&seed| {
            let mut r = LegacyRandom::new(seed);
            r.next_float() < 0.55 * special
        })
        .unwrap();
    game.mobs[index].random = EntityRandom::own(LegacyRandom::new(seed));
    let mut expected = game.mobs[index].random.clone();
    assert!(expected.next_float() < 0.55 * special);
    *game.held_mut() = Some(ItemStack::new("minecraft:zombie_spawn_egg", 1));
    assert!(game.spawn_egg_offspring(index));
    let child = game
        .mobs
        .iter()
        .find(|m| m.id != parent && m.species() == Species::Zombie)
        .unwrap();
    assert!(child.can_pick_up_loot());
    assert_eq!(
        game.mobs[index].random.next_float(),
        expected.next_float(),
        "the parent's random made the roll"
    );
}
