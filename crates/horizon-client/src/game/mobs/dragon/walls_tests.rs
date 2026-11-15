//! `can_entity_destroy` asked by the dragon's `checkWalls` and the wither's block break: a
//! block part decides what the entity may destroy, after the griefing gate and before
//! `living_destroy_block`.
use super::*;
use mod_api::{
    BlockPart, CanEntityDestroyEvent, Flow, Hooks, Host, LivingDestroyBlockEvent, Registrar,
    ServerHost,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

/// Answers whether any entity may destroy the block: always this answer.
struct May(bool);
#[mod_api::horizon_part]
impl BlockPart for May {
    fn can_entity_destroy(&self, _: &dyn Host, _: &mut CanEntityDestroyEvent) -> Option<bool> {
        Some(self.0)
    }
}
struct Walls {
    keep: Arc<AtomicBool>,
}
#[mod_api::__subscriptions]
impl Hooks for Walls {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        for (name, may) in [
            ("minecraft:obsidian", true),
            ("minecraft:glass", true),
            ("minecraft:bedrock", true),
            ("minecraft:stone", false),
        ] {
            let key = r.block_key(name);
            r.block_part(key, May(may));
        }
    }
    fn living_destroy_block(
        &self,
        _: &mut dyn ServerHost,
        _: &mut LivingDestroyBlockEvent<'_>,
    ) -> Flow {
        if self.keep.load(Ordering::Relaxed) {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
}

#[test]
#[ignore = "daily"]
fn block_parts_answer_what_the_dragon_and_the_wither_may_destroy() {
    let keep = Arc::new(AtomicBool::new(false));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "walls",
        Box::new(Walls { keep: keep.clone() }),
    )]
    .into();
    mod_platform::testing::with_mods(mods.clone(), || {
        crate::game::api_values::install_parts(&mods).unwrap();
        super::super::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
            game.dragon.discard();
            game.native_joins_skip();
            let id = game
                .spawn_mob_spawned(
                    "minecraft:ender_dragon",
                    [8.5, 70.0, 8.5],
                    super::super::creatures::SpawnSite::Finalizing(
                        horizon_client::mobs::spawn_rules::Reason::SpawnItemUse,
                    ),
                )
                .unwrap();
            game.place_dragon_parts();
            let registry = game.pack.assets.registry;
            let [obsidian, stone, bedrock, dirt, glass] = [
                "minecraft:obsidian",
                "minecraft:stone",
                "minecraft:bedrock",
                "minecraft:dirt",
                "minecraft:glass",
            ]
            .map(|name| registry.default_state(name).unwrap());
            // The chunk around the dragon, all of `state`; how many cells still are, after
            // `checkWalls`, and whether the dragon is in a wall.
            let walls = |game: &mut Game<'_>, state| {
                for x in 0..16 {
                    for y in 60..82 {
                        for z in 0..16 {
                            game.world.set(x, y, z, state).unwrap();
                        }
                    }
                }
                game.dragon_check_walls();
                let left = (0..16)
                    .flat_map(|x| (60..82).flat_map(move |y| (0..16).map(move |z| (x, y, z))))
                    .filter(|&(x, y, z)| game.world.block(x, y, z) == state)
                    .count();
                (16 * 22 * 16 - left, game.dragon.as_ref().unwrap().in_wall)
            };
            let griefing = |game: &mut Game<'_>, on: bool| {
                assert!(
                    game.ui
                        .rules
                        .set("mob_griefing", horizon_client::gamerules::Value::Bool(on),)
                );
            };
            griefing(game, true);
            // Glass answering yes (not #dragon_immune) breaks, and leaves no wall.
            let (broken, in_wall) = walls(game, glass);
            assert!(broken > 0 && !in_wall, "{broken} {in_wall}");
            // Obsidian is #dragon_immune: answering yes does not bypass it.
            assert_eq!(walls(game, obsidian), (0, true));
            // Stone answering no stays, and the dragon is in a wall.
            assert_eq!(walls(game, stone), (0, true));
            // A living_destroy_block cancel keeps a block the part allowed.
            keep.store(true, Ordering::Relaxed);
            assert_eq!(walls(game, glass), (0, true));
            keep.store(false, Ordering::Relaxed);
            // With mob_griefing off nothing breaks, whatever the part says.
            griefing(game, false);
            assert_eq!(walls(game, glass), (0, true));
            griefing(game, true);
            // The wither asks through its mob world: stone answering no is kept, and bedrock
            // (#wither_immune) answering yes breaks; an unanswered block keeps vanilla's rule.
            game.world.set(1, 64, 1, stone).unwrap();
            game.world.set(2, 64, 1, bedrock).unwrap();
            game.world.set(4, 64, 1, dirt).unwrap();
            let world = game.mob_world_base(&[]);
            assert!(!world.can_entity_destroy(id, [1, 64, 1], true));
            assert!(world.can_entity_destroy(id, [2, 64, 1], false));
            assert!(!world.can_entity_destroy(id, [4, 64, 1], false));
            assert!(world.can_entity_destroy(id, [4, 64, 1], true));
        });
    });
}
