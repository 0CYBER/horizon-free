#![cfg(test)]
use crate::game::{
    execution_role::ExecutionRole,
    native_entity_refs::tests::{run_fresh_cases, with_game_role},
};
use horizon_core::{
    geometry::Direction,
    position::{BlockPos, ChunkPos},
    random::LegacyRandom,
};
use horizon_world::{
    level::{Behaviours, BlockEntityNeighbors, LevelAccess, PushReaction as EngineReaction, Ticks},
    registry::{BlockRegistry, BlockStateId},
};
use mod_api::*;
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
};

struct ReactionPart {
    enabled: Arc<AtomicBool>,
    reaction: Option<PushReaction>,
    unbreakable: Option<Arc<AtomicU8>>,
}
#[mod_api::horizon_part]
impl BlockPart for ReactionPart {
    fn piston_push_reaction(&self, event: &mut PistonPushReactionEvent) -> Option<PushReaction> {
        if !self.enabled.load(Ordering::Relaxed) {
            return None;
        }
        if let Some(value) = &self.unbreakable {
            return Some(match value.load(Ordering::Relaxed) {
                0 => PushReaction::PushPull,
                1 => PushReaction::Push,
                2 => PushReaction::Popped,
                3 => PushReaction::Immoveable,
                4 => PushReaction::IgnoreEntity,
                _ => unreachable!(),
            });
        }
        self.reaction.clone().or_else(|| {
            Some(if event.state.ordinal() == 0 {
                PushReaction::Immoveable
            } else {
                PushReaction::PushPull
            })
        })
    }
}
struct User {
    enabled: Arc<AtomicBool>,
    parts: bool,
    unbreakable: Arc<AtomicU8>,
}
#[mod_api::__subscriptions]
impl Hooks for User {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        for (id, reaction) in [
            ("test:lock", None),
            ("test:fixed", Some(PushReaction::PushPull)),
            ("minecraft:chest", Some(PushReaction::Push)),
            ("minecraft:bedrock", Some(PushReaction::PushPull)),
        ] {
            let key = r.block_key(id);
            if self.parts {
                r.block_part(
                    key,
                    ReactionPart {
                        enabled: self.enabled.clone(),
                        reaction,
                        unbreakable: (id == "minecraft:bedrock").then(|| self.unbreakable.clone()),
                    },
                );
            }
        }
    }
    fn blocks(&self, _: &RegistrationContext, event: &mut BlocksEvent<'_>) {
        let mut block = Block::new("test:lock", "minecraft:stone");
        block
            .state_properties
            .insert("locked".into(), BlockStateProperty::Boolean);
        block.default_state.insert("locked".into(), "false".into());
        event.blocks.push(block);
        let mut fixed = Block::new("test:fixed", "minecraft:stone");
        fixed.properties.push_reaction = Some(PushReaction::Immoveable);
        event.blocks.push(fixed);
    }
}
struct Level {
    blocks: HashMap<BlockPos, BlockStateId>,
    air: BlockStateId,
    time: i64,
    random: LegacyRandom,
}
impl BlockEntityNeighbors for Level {}
impl LevelAccess for Level {
    fn block_state(&self, pos: BlockPos) -> BlockStateId {
        self.blocks.get(&pos).copied().unwrap_or(self.air)
    }
    fn set_raw(&mut self, pos: BlockPos, state: BlockStateId) -> Option<BlockStateId> {
        Some(self.blocks.insert(pos, state).unwrap_or(self.air))
    }
    fn is_loaded(&self, _: BlockPos) -> bool {
        true
    }
    fn random(&mut self) -> &mut LegacyRandom {
        &mut self.random
    }
    fn game_time(&self) -> i64 {
        self.time
    }
}
fn state(registry: &BlockRegistry, name: &str, props: &[(&str, &str)]) -> BlockStateId {
    registry
        .resolve(&horizon_world::chunk::BlockState {
            name: name.into(),
            properties: props
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        })
        .unwrap()
}
fn exercise(
    b: &Behaviours,
    registry: &BlockRegistry,
    block: BlockStateId,
    client: bool,
    moved: bool,
    popped: bool,
    pull: bool,
) {
    let base = BlockPos::new(0, 1, 0);
    let start = BlockPos::new(1, 1, 0);
    let end = BlockPos::new(2, 1, 0);
    let power_pos = BlockPos::new(-1, 1, 0);
    let piston = state(registry, "minecraft:sticky_piston", &[("facing", "east")]);
    let power = registry.default_state("minecraft:redstone_block").unwrap();
    let stone = registry.default_state("minecraft:stone").unwrap();
    let mut level = Level {
        blocks: HashMap::new(),
        air: registry.default_state("minecraft:air").unwrap(),
        time: 0,
        random: LegacyRandom::new(42),
    };
    let mut ticks = Ticks::new(0);
    for x in -1..=1 {
        for z in -1..=1 {
            ticks.add_empty_chunk(ChunkPos::new(x, z)).unwrap();
        }
    }
    for x in -2..=3 {
        level.blocks.insert(BlockPos::new(x, 0, 0), stone);
    }
    level.blocks.extend([(base, piston), (start, block)]);
    if client {
        level.blocks.insert(power_pos, power);
        b.client_block_event(&mut level, &mut ticks, base, 0, Direction::East as i32);
    } else {
        b.set_block(&mut level, &mut ticks, power_pos, power, 3);
    }
    for _ in 0..4 {
        level.time += 1;
        if client {
            b.client_tick_block_entities(&mut level, &mut ticks);
        } else {
            b.tick_level_blocks(&mut level, &mut ticks);
            b.tick_block_entities(&mut level, &mut ticks);
        }
    }
    assert_eq!(
        level.block_state(end),
        if moved {
            block
        } else {
            registry.default_state("minecraft:air").unwrap()
        },
        "extension destination"
    );
    if !moved && !popped {
        assert_eq!(level.block_state(start), block, "refused block stays");
    }
    if popped {
        assert_ne!(level.block_state(start), block, "bed pops");
    }
    if moved {
        if client {
            level.blocks.remove(&power_pos);
            b.client_block_event(&mut level, &mut ticks, base, 1, Direction::East as i32);
        } else {
            b.set_block(
                &mut level,
                &mut ticks,
                power_pos,
                registry.default_state("minecraft:air").unwrap(),
                3,
            );
        }
        for _ in 0..4 {
            level.time += 1;
            if client {
                b.client_tick_block_entities(&mut level, &mut ticks);
            } else {
                b.tick_level_blocks(&mut level, &mut ticks);
                b.tick_block_entities(&mut level, &mut ticks);
            }
        }
        assert_eq!(
            level.block_state(end),
            if pull {
                registry.default_state("minecraft:air").unwrap()
            } else {
                block
            },
            "sticky retraction destination"
        );
        if pull {
            assert_eq!(level.block_state(start), block);
        }
    }
}
#[test]
#[ignore = "daily"]
fn piston_push_reaction_parts_cache_reload_and_replay() {
    const NAME: &str = "game::native_hooks::piston_push_reaction_tests::piston_push_reaction_parts_cache_reload_and_replay";
    const FLAG: &str = "HORIZON_PISTON_REACTION_CASE";
    if run_fresh_cases(NAME, FLAG, &["server", "client", "no-part"]) {
        return;
    }
    let case = std::env::var(FLAG).unwrap();
    let client = case == "client";
    let parts = case != "no-part";
    let enabled = Arc::new(AtomicBool::new(true));
    let unbreakable = Arc::new(AtomicU8::new(0));
    let mods: Mods = vec![ModEntry::new(
        "test",
        Box::new(User {
            enabled: enabled.clone(),
            parts,
            unbreakable: unbreakable.clone(),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods.clone(), || {
        crate::game::api_values::install_parts(&mods).unwrap();
        let base = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let root = crate::game::mods::content_root(&base, &Default::default()).unwrap();
        with_game_role(
            root.prepared().to_path_buf(),
            if client {
                ExecutionRole::Client
            } else {
                ExecutionRole::IntegratedServer
            },
            |game| {
                let registry = game.pack.assets.registry;
                let a = state(registry, "test:lock", &[("locked", "false")]);
                let z = state(registry, "test:lock", &[("locked", "true")]);
                assert_eq!(
                    game.res.behaviours().piston_reaction(a),
                    if parts {
                        EngineReaction::Immoveable
                    } else {
                        EngineReaction::PushPull
                    }
                );
                exercise(
                    game.res.behaviours(),
                    registry,
                    a,
                    client,
                    !parts,
                    false,
                    true,
                );
                exercise(
                    game.res.behaviours(),
                    registry,
                    z,
                    client,
                    true,
                    false,
                    true,
                );
                let fixed = registry.default_state("test:fixed").unwrap();
                exercise(
                    game.res.behaviours(),
                    registry,
                    fixed,
                    client,
                    parts,
                    false,
                    true,
                );
                if parts {
                    let chest = registry.default_state("minecraft:chest").unwrap();
                    exercise(
                        game.res.behaviours(),
                        registry,
                        chest,
                        client,
                        false,
                        false,
                        false,
                    );
                    let bedrock = registry.default_state("minecraft:bedrock").unwrap();
                    // Every reaction loses to the hardness refusal, including popped.
                    for reaction in 0..5 {
                        unbreakable.store(reaction, Ordering::Relaxed);
                        game.resolve_parts().unwrap();
                        exercise(
                            game.res.behaviours(),
                            registry,
                            bedrock,
                            client,
                            false,
                            false,
                            false,
                        );
                    }
                }
                // Removing the last subscriber must clear its immovable override on both sides.
                if parts {
                    unbreakable.store(0, Ordering::Relaxed);
                    assert_eq!(
                        game.res.behaviours().piston_reaction(a),
                        EngineReaction::Immoveable
                    );
                }
                // A reload retains registry metadata; only the active subscriber set changes.
                let after: Mods = if parts {
                    Vec::new().into()
                } else {
                    mods.clone()
                };
                mod_platform::testing::with_mods(after, || {
                    if parts {
                        game.resolve_parts().unwrap();
                        assert!(!mod_api::__private::has_scoped_hook(
                            HookId::PistonPushReaction
                        ));
                        assert_eq!(
                            game.res.behaviours().piston_reaction(a),
                            EngineReaction::PushPull
                        );
                        exercise(
                            game.res.behaviours(),
                            registry,
                            a,
                            client,
                            true,
                            false,
                            true,
                        );
                    }
                    // Reinstall the actual resolved parts as reload does. None restores properties.
                    enabled.store(false, Ordering::Relaxed);
                    game.resolve_parts().unwrap();
                    assert_eq!(
                        game.res.behaviours().piston_reaction(a),
                        EngineReaction::PushPull
                    );
                    for (name, moved, popped) in [
                        ("minecraft:stone", true, false),
                        ("minecraft:obsidian", false, false),
                        ("minecraft:red_bed", false, true),
                        ("test:lock", true, false),
                        ("test:fixed", false, false),
                        ("minecraft:chest", false, false),
                    ] {
                        let block = registry.default_state(name).unwrap();
                        exercise(
                            game.res.behaviours(),
                            registry,
                            block,
                            client,
                            moved,
                            popped,
                            true,
                        );
                    }
                });
            },
        );
    });
}

struct PayloadPart {
    reaction: PushReaction,
    carry: Option<bool>,
    asks: Arc<std::sync::atomic::AtomicUsize>,
}
#[mod_api::horizon_part]
impl BlockPart for PayloadPart {
    fn piston_push_reaction(&self, _: &mut PistonPushReactionEvent) -> Option<PushReaction> {
        Some(self.reaction.clone())
    }
    fn piston_carries_block_entity(
        &self,
        _: LevelSide<'_>,
        _: &mut PistonCarriesBlockEntityEvent,
    ) -> Option<bool> {
        self.asks.fetch_add(1, Ordering::Relaxed);
        self.carry
    }
}
struct PayloadUser {
    reaction: PushReaction,
    carry: Option<bool>,
    asks: Arc<std::sync::atomic::AtomicUsize>,
}
#[mod_api::__subscriptions]
impl Hooks for PayloadUser {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        for name in [
            "minecraft:chest",
            "minecraft:furnace",
            "minecraft:hopper",
            "minecraft:comparator",
        ] {
            let key = r.block_key(name);
            r.block_part(
                key,
                PayloadPart {
                    reaction: self.reaction.clone(),
                    carry: self.carry,
                    asks: self.asks.clone(),
                },
            );
        }
    }
}

#[test]
#[ignore = "daily"]
fn piston_push_reaction_real_block_entity_payload_lifecycle() {
    const NAME: &str = "game::native_hooks::piston_push_reaction_tests::piston_push_reaction_real_block_entity_payload_lifecycle";
    const FLAG: &str = "HORIZON_PISTON_PAYLOAD_CASE";
    if run_fresh_cases(
        NAME,
        FLAG,
        &[
            "push-refused",
            "push-carry",
            "push-false",
            "push-pull-refused",
            "push-pull-carry",
            "ignore-refused",
            "ignore-carry",
            "popped",
            "immovable-carry",
        ],
    ) {
        return;
    }
    let case = std::env::var(FLAG).unwrap();
    let popped = case == "popped";
    let carry = case.ends_with("-carry");
    let moves = carry && case != "immovable-carry";
    let reaction = if case == "immovable-carry" {
        PushReaction::Immoveable
    } else if popped {
        PushReaction::Popped
    } else if case.starts_with("push-pull") {
        PushReaction::PushPull
    } else if case.starts_with("ignore") {
        PushReaction::IgnoreEntity
    } else {
        PushReaction::Push
    };
    let asks = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mods: Mods = vec![ModEntry::new(
        "payload",
        Box::new(PayloadUser {
            reaction,
            carry: if carry {
                Some(true)
            } else if case == "push-false" {
                Some(false)
            } else {
                None
            },
            asks: asks.clone(),
        }),
    )]
    .into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        crate::game::native_entity_refs::tests::with_game(|game| {
            use horizon_client::{chunk::Chunk, containers::BlockEntity};
            use horizon_items::items::ItemStack;
            game.options.dedicated = Some(vec![]);
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.block_ticks
                .add_empty_chunk(ChunkPos::new(0, 0))
                .unwrap();
            let registry = game.pack.assets.registry;
            let piston = state(registry, "minecraft:sticky_piston", &[("facing", "east")]);
            let chest = registry.default_state("minecraft:chest").unwrap();
            let power = registry.default_state("minecraft:redstone_block").unwrap();
            game.with_level(|_, level, _| {
                level.set_raw(BlockPos::new(2, 64, 2), piston);
                level.set_raw(BlockPos::new(1, 64, 2), power);
                level.set_raw(BlockPos::new(3, 64, 2), chest);
            });
            let mut slots = vec![None; 27];
            slots[4] = Some(ItemStack {
                item: "minecraft:diamond".into(),
                count: 7,
                damage: 0,
                extra: None,
            });
            game.world
                .block_entities
                .insert((3, 64, 2), BlockEntity::Chest(slots.clone()));
            assert!(
                matches!(game.world.block_entities.get(&(3,64,2)), Some(BlockEntity::Chest(before)) if before == &slots)
            );
            assert_eq!(game.world.block_entities.iter_mut().count(), 1);
            assert!(game.dropped.is_empty());
            game.with_level(|b, level, ticks| {
                b.client_block_event(
                    level,
                    ticks,
                    BlockPos::new(2, 64, 2),
                    0,
                    Direction::East as i32,
                );
            });
            if moves {
                assert_eq!(game.world.block_entities.iter_mut().count(), 0);
                let carried = game
                    .world
                    .block_entities
                    .carried(&(4, 64, 2))
                    .expect("one carried chest");
                assert!(matches!(&carried.entity, BlockEntity::Chest(during) if during == &slots));
            }
            for _ in 0..4 {
                game.world.game_time += 1;
                game.with_level(|b, level, ticks| b.client_tick_block_entities(level, ticks));
            }
            assert!(game.block_ticks.moving_pistons.is_empty());
            assert!(game.world.block_entities.carried(&(4, 64, 2)).is_none());
            assert_eq!(
                game.world.block_entities.iter_mut().count(),
                if popped { 0 } else { 1 }
            );
            if popped {
                assert_eq!(
                    asks.load(Ordering::Relaxed),
                    0,
                    "popped never asks the carry hook"
                );
                assert_eq!(
                    game.world.block(4, 64, 2),
                    registry.default_state("minecraft:air").unwrap()
                );
                assert!(game.world.block_entities.get(&(3, 64, 2)).is_none());
            } else {
                if case == "immovable-carry" {
                    assert_eq!(asks.load(Ordering::Relaxed), 0);
                } else {
                    assert!(asks.load(Ordering::Relaxed) > 0);
                }
                let destination = if moves { (4, 64, 2) } else { (3, 64, 2) };
                assert_eq!(
                    game.world
                        .block(destination.0, destination.1, destination.2),
                    chest
                );
                let Some(BlockEntity::Chest(after)) = game.world.block_entities.get(&destination)
                else {
                    panic!("one intact chest")
                };
                assert_eq!(
                    after, &slots,
                    "all 27 slots, seven diamonds in slot 4, damage 0 and no extra fields survive"
                );
                let other = if moves { (3, 64, 2) } else { (4, 64, 2) };
                assert!(game.world.block_entities.get(&other).is_none());
            }
            if case == "push-carry" {
                let before = asks.load(Ordering::Relaxed);
                game.with_level(|b, level, ticks| {
                    level.set_raw(
                        BlockPos::new(1, 64, 2),
                        registry.default_state("minecraft:air").unwrap(),
                    );
                    b.client_block_event(
                        level,
                        ticks,
                        BlockPos::new(2, 64, 2),
                        1,
                        Direction::East as i32,
                    );
                });
                for _ in 0..4 {
                    game.world.game_time += 1;
                    game.with_level(|b, level, ticks| b.client_tick_block_entities(level, ticks));
                }
                assert_eq!(
                    asks.load(Ordering::Relaxed),
                    before,
                    "wrong-direction push only never asks carry"
                );
                assert!(
                    matches!(game.world.block_entities.get(&(4,64,2)), Some(BlockEntity::Chest(after)) if after == &slots)
                );
                assert_eq!(game.world.block_entities.iter_mut().count(), 1);
            }
            let diamonds: Vec<_> = game
                .dropped
                .iter()
                .filter(|e| &*e.stack.item == "minecraft:diamond")
                .collect();
            assert_eq!(
                diamonds.iter().map(|e| e.stack.count).sum::<u32>(),
                if popped { 7 } else { 0 }
            );
            for drop in diamonds {
                assert_eq!(drop.stack.damage, 0);
                assert_eq!(drop.stack.extra, None);
            }
        });
    });
}

struct TagUser;
#[mod_api::__subscriptions]
impl Hooks for TagUser {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        r.block_part_tag(
            "test:anchored",
            ReactionPart {
                enabled: Arc::new(AtomicBool::new(true)),
                reaction: Some(PushReaction::Immoveable),
                unbreakable: None,
            },
        );
    }
}
#[test]
#[ignore = "daily"]
fn piston_push_reaction_client_received_tags_refresh_the_cache() {
    const NAME: &str = "game::native_hooks::piston_push_reaction_tests::piston_push_reaction_client_received_tags_refresh_the_cache";
    if run_fresh_cases(NAME, "HORIZON_PISTON_TAG_CASE", &["client"]) {
        return;
    }
    let mods: Mods = vec![ModEntry::new("tagged", Box::new(TagUser))].into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        crate::game::native_entity_refs::tests::with_client_game(|game| {
            use horizon_protocol::packets::configuration::{RegistryTags, TagEntry, UpdateTags};
            let (pipe, _server) = horizon_protocol::connection::Pipe::pair();
            game.connect_local(pipe, "PistonTags").unwrap();
            let registry = game.pack.assets.registry;
            let stone = registry.default_state("minecraft:stone").unwrap();
            let protocol_stone = game
                .remote
                .as_ref()
                .unwrap()
                .builtin
                .get("minecraft:block")
                .unwrap()
                .id("minecraft:stone")
                .unwrap();
            let packet = |ids| UpdateTags {
                registries: vec![RegistryTags {
                    registry: "minecraft:block".into(),
                    tags: vec![TagEntry {
                        name: "test:anchored".into(),
                        ids,
                    }],
                }],
            };
            // Configuration's tags become active at login through install_remote_tags.
            Arc::make_mut(&mut game.remote.as_mut().unwrap().registries)
                .add_tags(&packet(vec![protocol_stone]));
            game.install_remote_tags();
            assert_eq!(
                game.res.behaviours().piston_reaction(stone),
                EngineReaction::Immoveable
            );
            exercise(
                game.res.behaviours(),
                registry,
                stone,
                true,
                false,
                false,
                false,
            );
            // A reload's play-state tag packet removes the attachment, restoring stone's properties.
            game.remote_update_tags(&packet(vec![]));
            assert_eq!(
                game.res.behaviours().piston_reaction(stone),
                EngineReaction::PushPull
            );
            exercise(
                game.res.behaviours(),
                registry,
                stone,
                true,
                true,
                false,
                true,
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn piston_push_reaction_position_state_opens_ticks_and_saves() {
    const NAME: &str = "game::native_hooks::piston_push_reaction_tests::piston_push_reaction_position_state_opens_ticks_and_saves";
    const FLAG: &str = "HORIZON_PISTON_POSITION_CASE";
    if run_fresh_cases(
        NAME,
        FLAG,
        &[
            "chest",
            "furnace",
            "hopper",
            "client",
            "loot",
            "mixed-loot",
            "same-scope",
        ],
    ) {
        return;
    }
    let case = std::env::var(FLAG).unwrap();
    let client = case == "client";
    let asks = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mods: Mods = vec![ModEntry::new(
        "payload",
        Box::new(PayloadUser {
            reaction: PushReaction::Push,
            carry: Some(true),
            asks,
        }),
    )]
    .into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        with_game_role(
            root,
            if client {
                ExecutionRole::Client
            } else {
                ExecutionRole::IntegratedServer
            },
            |game| {
                use horizon_client::{
                    chunk::{Chunk, PendingContainerLoot},
                    containers::{BlockEntity, Furnace, FurnaceKind},
                    items::ItemStack,
                };
                use horizon_core::nbt::Tag;
                if client {
                    let (pipe, _server) = horizon_protocol::connection::Pipe::pair();
                    game.connect_local(pipe, "CarryReplay").unwrap();
                } else {
                    game.options.dedicated = Some(vec![]);
                }
                let from = (3, 64, 2);
                let to = (4, 64, 2);
                let registry = game.pack.assets.registry;
                let name = match case.as_str() {
                    "furnace" => "minecraft:furnace",
                    "hopper" | "same-scope" => "minecraft:hopper",
                    _ => "minecraft:chest",
                };
                let block = registry.default_state(name).unwrap();
                let piston = state(registry, "minecraft:piston", &[("facing", "east")]);
                let power = registry.default_state("minecraft:redstone_block").unwrap();
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                game.block_ticks
                    .add_empty_chunk(ChunkPos::new(0, 0))
                    .unwrap();
                game.with_level(|_, level, _| {
                    level.set_raw(BlockPos::new(2, 64, 2), piston);
                    level.set_raw(BlockPos::new(1, 64, 2), power);
                    level.set_raw(BlockPos::new(3, 64, 2), block);
                });
                let mut slots = vec![
                    None;
                    if case == "hopper" || case == "same-scope" {
                        5
                    } else {
                        27
                    }
                ];
                if case != "loot" {
                    slots[4] = Some(ItemStack::new("minecraft:diamond", 7));
                }
                let entity = if case == "furnace" {
                    BlockEntity::Furnace(Furnace {
                        kind: FurnaceKind::Furnace,
                        items: [None, None, None],
                        lit_remaining: 100,
                        lit_total: 100,
                        cooking_timer: 0,
                        cooking_total: 200,
                        speed: 1.0,
                        recipes_used: HashMap::new(),
                    })
                } else {
                    BlockEntity::Chest(slots.clone())
                };
                game.world.block_entities.insert(from, entity);
                let old_ticker = if case == "furnace" {
                    let states = game.ticking_states();
                    game.world.block_entities.set_states(states);
                    let chunks = &game.world.chunks;
                    let tickers = game.world.block_entities.tickers(|p| {
                        chunks
                            .get(&(p.0 >> 4, p.2 >> 4))
                            .unwrap()
                            .block(p.0 & 15, p.1, p.2 & 15)
                            .0
                    });
                    Some(*tickers.iter().find(|(p, _)| *p == from).unwrap())
                } else {
                    None
                };
                let custom = Tag::String("Moving chest".into());
                game.world.block_names.insert(from, custom.clone());
                let lock = Tag::compound([("items", Tag::String("minecraft:diamond".into()))]);
                game.world.set_block_lock(
                    from,
                    &Tag::compound([
                        ("id", Tag::String(name.into())),
                        ("x", Tag::Int(3)),
                        ("y", Tag::Int(64)),
                        ("z", Tag::Int(2)),
                        ("lock", lock.clone()),
                        (
                            "components",
                            Tag::compound([(
                                "minecraft:custom_data",
                                Tag::compound([("carry_test", Tag::Int(11))]),
                            )]),
                        ),
                    ]),
                );
                game.world.set_container_stack_limit(from, 13);
                if case == "loot" || case == "mixed-loot" {
                    let files = [("loot_table/test:carry".into(), vec![br#"{"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:diamond"}]}]}"#.to_vec()])];
                    let loot = horizon_world::loot::LootData::load(
                        &game.options.resources,
                        &horizon_world::loot::PackOverlay {
                            files: &files,
                            ..horizon_world::loot::PackOverlay::NONE
                        },
                        game.res.item_defaults(),
                    )
                    .unwrap();
                    game.server_loot.set(Some(Arc::new(loot)));
                    let mut extras = game.world.block_entity_extras(from);
                    extras.pending_loot = Some(PendingContainerLoot {
                        table: Tag::String("test:carry".into()),
                        seed: Some(Tag::Long(37)),
                    });
                    game.world.set_block_entity_extras(from, extras);
                }
                if case == "hopper" || case == "same-scope" {
                    game.hoppers
                        .insert(from, crate::game::machines::HopperState::with_cooldown(40));
                }
                game.lid_openers.insert(from, 2);
                game.world.chest_openers.insert(from, (2, 4.0));
                game.be_anims.chests.insert(
                    [3, 64, 2],
                    horizon_client::block_entity_anim::ChestLid {
                        should_be_open: true,
                        openness: 0.7,
                        o_openness: 0.6,
                    },
                );
                game.container = Some(from);
                game.open_container_slots = Some((from, 27));
                if case == "same-scope" {
                    let far = (5, 64, 2);
                    let air = registry.default_state("minecraft:air").unwrap();
                    game.with_level(|_, level, _| {
                        level.carry_mod_block_entity(
                            BlockPos::new(3, 64, 2),
                            BlockPos::new(4, 64, 2),
                        );
                        level.set_raw(BlockPos::new(3, 64, 2), air);
                        level.set_raw(BlockPos::new(4, 64, 2), block);
                        assert!(level.land_carried_block_entity(BlockPos::new(4, 64, 2)));
                        level.carry_mod_block_entity(
                            BlockPos::new(4, 64, 2),
                            BlockPos::new(5, 64, 2),
                        );
                        level.set_raw(BlockPos::new(4, 64, 2), air);
                        level.set_raw(BlockPos::new(5, 64, 2), block);
                        assert!(level.land_carried_block_entity(BlockPos::new(5, 64, 2)));
                    });
                    assert_eq!(game.hopper_cooldown(far), 40);
                    assert!(!game.hoppers.contains_key(&from) && !game.hoppers.contains_key(&to));
                    assert!(
                        matches!(game.world.block_entities.get(&far),Some(BlockEntity::Chest(after)) if after==&slots)
                    );
                    assert_eq!(game.world.block_names.get(&far), Some(&custom));
                    assert_eq!(game.world.block_lock(far), Some(&lock));
                    assert_eq!(game.world.container_stack_limit(far), Some(13));
                    assert_eq!(game.world.block_entities.iter_mut().count(), 1);
                    return;
                }
                game.with_level(|b, level, ticks| {
                    b.client_block_event(
                        level,
                        ticks,
                        BlockPos::new(2, 64, 2),
                        0,
                        Direction::East as i32,
                    );
                });
                assert!(
                    game.container.is_none(),
                    "source menu closes rather than following another holder"
                );
                assert!(game.world.block_entities.get(&from).is_none());
                assert!(!game.world.block_names.contains_key(&from));
                assert!(game.world.block_lock(from).is_none());
                assert_eq!(game.world.container_stack_limit(from), None);
                assert!(!game.lid_openers.contains_key(&from));
                assert!(!game.world.chest_openers.contains_key(&from));
                assert!(!game.be_anims.chests.contains_key(&[3, 64, 2]));
                // Save/reload in flight also keeps the position-owned fields.
                let at = BlockPos::new(4, 64, 2);
                let moving = game
                    .block_ticks
                    .moving_pistons
                    .iter()
                    .find(|(p, _)| *p == at)
                    .unwrap()
                    .1;
                let saved = game.saved_moving_piston(at, &moving);
                game.world.block_entities.discard_carried(&to);
                let mut extras = horizon_client::save::ChunkExtras::default();
                extras.block_entities.push(saved);
                game.restore_moving_piston(at, moving, Some(&extras));
                for _ in 0..10 {
                    game.world.game_time += 1;
                    game.with_level(|b, level, ticks| b.client_tick_block_entities(level, ticks));
                }
                assert_eq!(game.world.block_entities.iter_mut().count(), 1);
                assert!(game.world.block_entities.get(&from).is_none());
                assert_eq!(game.world.block_names.get(&to), Some(&custom));
                assert_eq!(game.world.block_lock(to), Some(&lock));
                assert_eq!(game.world.container_stack_limit(to), Some(13));
                assert_eq!(
                    game.world
                        .block_entity_extras(to)
                        .source
                        .unwrap()
                        .get("components")
                        .and_then(|t| t.get("minecraft:custom_data"))
                        .and_then(|t| t.get("carry_test")),
                    Some(&Tag::Int(11))
                );
                if case == "furnace" {
                    assert!(
                        matches!(game.world.block_entities.get(&to),Some(BlockEntity::Furnace(f)) if f.lit_remaining==100)
                    );
                    let states = game.ticking_states();
                    game.world.block_entities.set_states(states);
                    let chunks = &game.world.chunks;
                    let slots = game.world.block_entities.tickers(|p| {
                        chunks
                            .get(&(p.0 >> 4, p.2 >> 4))
                            .unwrap()
                            .block(p.0 & 15, p.1, p.2 & 15)
                            .0
                    });
                    assert!(slots.iter().any(|(p, _)| *p == to));
                    assert!(!game.world.block_entities.is_live(old_ticker.unwrap()));
                    assert!(!slots.iter().any(|(p, _)| *p == from));
                    game.tick_furnace(to).unwrap();
                    assert!(
                        matches!(game.world.block_entities.get(&to),Some(BlockEntity::Furnace(f)) if f.lit_remaining==99)
                    );
                } else {
                    assert!(
                        matches!(game.world.block_entities.get(&to),Some(BlockEntity::Chest(after)) if after==&slots)
                    );
                    if case == "hopper" {
                        assert!(!game.hoppers.contains_key(&from));
                        assert_eq!(game.hopper_cooldown(to), 40);
                    } else if case != "loot" && case != "mixed-loot" {
                        game.spectator_open_menu([4, 64, 2]);
                        assert_eq!(game.container, Some(to));
                        game.be_anims.classify(registry);
                        game.block_entity_event([4, 64, 2], 1, 1);
                        game.tick_chest_lid([4, 64, 2]);
                        assert_eq!(game.be_anims.chests[&[4, 64, 2]].openness, 0.1);
                        assert!(!game.be_anims.chests.contains_key(&[3, 64, 2]));
                    }
                }
                let tag = game.block_entity_nbt([4, 64, 2]).unwrap();
                assert_eq!(tag.get("x"), Some(&Tag::Int(4)));
                assert!(game.block_entity_nbt([3, 64, 2]).is_none());
                if case == "mixed-loot" {
                    assert_eq!(
                        tag.get("LootTable"),
                        Some(&Tag::String("test:carry".into()))
                    );
                    assert_eq!(tag.get("LootTableSeed"), Some(&Tag::Long(37)));
                    game.build_registries();
                    assert!(game.as_console(|g| g.run_command(
                        "execute if items block 4 64 2 container.* minecraft:diamond run say mixed"
                    )));
                    assert!(game.world.block_entity_extras(to).pending_loot.is_none());
                    assert_eq!(
                        game.world.block_entities.get(&to).unwrap().slots(),
                        slots.as_slice()
                    );
                }
                let mut batch = horizon_client::save::chunks_with_block_entities(
                    game.world.chunks.values(),
                    &game.world.block_entities,
                    &game.world.block_names,
                );
                game.attach_save_data(&mut batch[0]);
                let encoded = horizon_client::save::encode(&batch[0], registry, game.res.biomes());
                let bytes =
                    horizon_core::nbt::write_named(&encoded, horizon_core::nbt::Limits::saved())
                        .unwrap();
                let (encoded, used) =
                    horizon_core::nbt::read_named(&bytes, horizon_core::nbt::Limits::saved())
                        .unwrap();
                assert_eq!(used, bytes.len());
                let Tag::List(saved_entities) = encoded.tag.get("block_entities").unwrap() else {
                    panic!()
                };
                let tag = saved_entities
                    .iter()
                    .find(|t| t.get("x") == Some(&Tag::Int(4)))
                    .unwrap();
                assert_eq!(
                    tag.get("components")
                        .and_then(|t| t.get("minecraft:custom_data"))
                        .and_then(|t| t.get("carry_test")),
                    Some(&Tag::Int(11))
                );
                assert_eq!(tag.get("CustomName"), Some(&custom));
                assert_eq!(tag.get("lock"), Some(&lock));
                if case == "loot" {
                    let roll = game.world.block_entity_extras(to).pending_loot.unwrap();
                    assert_eq!(roll.seed, Some(Tag::Long(37)));

                    assert_eq!(
                        tag.get("LootTable"),
                        Some(&Tag::String("test:carry".into()))
                    );
                    assert_eq!(tag.get("LootTableSeed"), Some(&Tag::Long(37)));
                } else if case == "furnace" {
                    let (_, BlockEntity::Furnace(saved_furnace)) =
                        horizon_client::save::decode_block_entity(tag).unwrap()
                    else {
                        panic!()
                    };
                    assert_eq!(saved_furnace.lit_remaining, 99);
                } else {
                    let (_, BlockEntity::Chest(saved_slots)) =
                        horizon_client::save::decode_block_entity(tag).unwrap()
                    else {
                        panic!()
                    };
                    assert_eq!(saved_slots, slots);
                }
                if matches!(case.as_str(), "loot" | "mixed-loot" | "chest") {
                    let loaded =
                        horizon_client::save::decode(&encoded, registry, game.res.biomes())
                            .unwrap();
                    assert!(
                        !loaded
                            .saved
                            .as_ref()
                            .unwrap()
                            .pending_loot
                            .contains_key(&from)
                    );
                    let (_, entity) = loaded
                        .block_entities
                        .iter()
                        .find(|(p, _)| *p == to)
                        .unwrap();
                    assert_eq!(entity.slots(), slots.as_slice());
                    game.world.block_entities.remove(&to);
                    game.world.block_entities.insert(to, entity.clone());
                    game.world.chunks.insert((0, 0), Arc::new(loaded));
                    game.build_registries();
                    for _ in 0..2 {
                        assert!(game.as_console(|g|g.run_command("execute if items block 4 64 2 container.* minecraft:diamond run say carried")));
                        assert!(game.world.block_entity_extras(to).pending_loot.is_none());
                        let count: u32 = game
                            .world
                            .block_entities
                            .get(&to)
                            .unwrap()
                            .slots()
                            .iter()
                            .flatten()
                            .filter(|s| &*s.item == "minecraft:diamond")
                            .map(|s| s.count)
                            .sum();
                        assert_eq!(count, if case == "loot" { 1 } else { 7 });
                    }
                }
            },
        );
    });
}

#[test]
#[ignore = "daily"]
fn piston_push_reaction_unowned_vanilla_data_stays_refused() {
    const NAME: &str = "game::native_hooks::piston_push_reaction_tests::piston_push_reaction_unowned_vanilla_data_stays_refused";
    if run_fresh_cases(NAME, "HORIZON_PISTON_UNOWNED_CASE", &["comparator"]) {
        return;
    }
    let mods: Mods = vec![ModEntry::new(
        "payload",
        Box::new(PayloadUser {
            reaction: PushReaction::Push,
            carry: Some(true),
            asks: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }),
    )]
    .into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        crate::game::native_entity_refs::tests::with_game(|game| {
            use horizon_client::chunk::Chunk;
            game.options.dedicated = Some(vec![]);
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.block_ticks
                .add_empty_chunk(ChunkPos::new(0, 0))
                .unwrap();
            let registry = game.pack.assets.registry;
            let comparator = registry.default_state("minecraft:comparator").unwrap();
            let piston = state(registry, "minecraft:piston", &[("facing", "east")]);
            let power = registry.default_state("minecraft:redstone_block").unwrap();
            game.with_level(|_, level, _| {
                level.set_raw(BlockPos::new(2, 64, 2), piston);
                level.set_raw(BlockPos::new(1, 64, 2), power);
                level.set_raw(BlockPos::new(3, 64, 2), comparator);
            });
            game.block_ticks
                .comparator_outputs
                .insert(BlockPos::new(3, 64, 2), 9);
            game.with_level(|b, level, ticks| {
                b.client_block_event(
                    level,
                    ticks,
                    BlockPos::new(2, 64, 2),
                    0,
                    Direction::East as i32,
                );
            });
            assert_eq!(game.world.block(3, 64, 2), comparator);
            assert_eq!(
                game.block_ticks
                    .comparator_outputs
                    .get(&BlockPos::new(3, 64, 2)),
                Some(&9)
            );
            assert!(game.block_ticks.moving_pistons.is_empty());
        })
    });
}
