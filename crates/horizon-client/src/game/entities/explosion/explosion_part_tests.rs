use super::*;
use mod_api::{
    BlockPart, BreakBlockEvent, CanDropFromExplosionEvent, CanEntityDestroyEvent, ExpDropEvent,
    ExplosionResistanceEvent, Flow, Hooks, LevelHost, OnBlockExplodedEvent, Registrar, ServerHost,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicI32, Ordering},
};

#[derive(Clone)]
struct Xp(Arc<AtomicI32>);
#[mod_api::horizon_part]
impl BlockPart for Xp {
    fn exp_drop(&self, level: &dyn LevelHost, event: &mut ExpDropEvent<'_>) -> Option<i32> {
        if level.state_block(event.state) == Some("minecraft:chest") {
            let data = level
                .block_entity_data(event.block_entity.expect("live XP block entity"))
                .unwrap();
            assert!(
                matches!(data.get(&"Items".into()), Some(mod_api::Tag::List(items)) if items.len() == 1)
            );
        }
        Some(self.0.load(Ordering::Relaxed))
    }
}
#[derive(Clone, Default)]
struct Blast(Arc<Mutex<Vec<[i32; 3]>>>, Arc<AtomicI32>);
#[mod_api::horizon_part]
impl BlockPart for Blast {
    fn on_block_exploded(&self, host: &mut dyn ServerHost, event: &mut OnBlockExplodedEvent<'_>) {
        if self.1.load(Ordering::Relaxed) == 1 {
            panic!("adversarial part failure");
        }
        if self.1.load(Ordering::Relaxed) == 3 {
            std::panic::panic_any("native trap-equivalent refusal");
        }
        let pos = [event.pos.x, event.pos.y, event.pos.z];
        let entity = event.block_entity.expect("live chest before removal");
        let data = entity.data(host).expect("readable live chest");
        let mod_api::Tag::List(items) = data.get(&"Items".into()).unwrap() else {
            panic!("chest items");
        };
        assert_eq!(items.len(), 1);
        let mod_api::Tag::Compound(item) = &items[0] else {
            panic!("saved stack");
        };
        assert_eq!(item.get(&"count".into()), Some(&mod_api::Tag::Int(3)));
        assert_eq!(event.explosion.center, [1.0, 65.0, 1.0]);
        assert_eq!(event.explosion.radius, 4.0);
        let level = host.level(event.explosion.dimension).unwrap();
        let state = host.block_state_id("minecraft:stone").unwrap();
        assert!(
            level
                .set_block(host, pos, state, mod_api::BlockUpdateFlags::CLIENTS)
                .unwrap()
        );
        if self.1.load(Ordering::Relaxed) == 2 {
            panic!("failure after reentrant replacement");
        }
        self.0.lock().unwrap().push(pos);
    }
}
struct Parts {
    xp: Xp,
    blast: Blast,
    add: Arc<AtomicBool>,
    seen: Arc<Mutex<Vec<Option<u32>>>>,
    cancel_blast: Arc<AtomicBool>,
}
#[mod_api::__subscriptions]
impl Hooks for Parts {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        for name in [
            "minecraft:diamond_ore",
            "minecraft:spawner",
            "minecraft:chest",
        ] {
            let key = r.block_key(name);
            r.block_part(key, self.xp.clone());
        }
        let key = r.block_key("minecraft:chest");
        r.block_part(key, self.blast.clone());
    }
    fn explosion_start(
        &self,
        _: &mut dyn ServerHost,
        event: &mut mod_api::ExplosionStartEvent<'_>,
    ) -> Flow {
        let _ = event;
        if self.cancel_blast.load(Ordering::Relaxed) {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
    fn break_block(&self, _: &mut dyn ServerHost, event: &mut BreakBlockEvent<'_>) -> Flow {
        self.seen.lock().unwrap().push(*event.exp_to_drop);
        if self.add.load(Ordering::Relaxed) {
            *event.exp_to_drop = Some(event.exp_to_drop.unwrap() + 3);
        }
        Flow::Continue
    }
}

#[test]
#[ignore = "needs prepared resources and isolated native mod state"]
fn block_parts_xp_and_explosion_contract() {
    let amount = Arc::new(AtomicI32::new(7));
    let add = Arc::new(AtomicBool::new(false));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let blast = Blast::default();
    let cancel_blast = Arc::new(AtomicBool::new(false));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "block_parts",
        Box::new(Parts {
            xp: Xp(amount.clone()),
            blast: blast.clone(),
            add: add.clone(),
            seen: seen.clone(),
            cancel_blast: cancel_blast.clone(),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods.clone(), || {
        crate::game::api_values::install_parts(&mods).unwrap();
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let ore = registry.default_state("minecraft:diamond_ore").unwrap();
            let stone = registry.default_state("minecraft:stone").unwrap();
            let spawner = registry.default_state("minecraft:spawner").unwrap();
            let pos = BlockPos::new(1, 64, 1);
            let drop = |game: &mut Game<'_>,
                        state,
                        tool: Option<&horizon_client::items::ItemStack>,
                        override_xp| {
                game.with_level(|_, level, _| {
                    level.block_dropped(pos, state, None, tool, Vec::new(), override_xp)
                })
                .experience
            };
            let emerald = registry.default_state("minecraft:emerald_ore").unwrap();
            game.world.random = horizon_core::random::LevelRandom::for_level(LegacyRandom::new(7));
            assert_eq!(drop(game, emerald, None, None), 4);
            assert_eq!(drop(game, ore, None, None), 7);
            assert_eq!(drop(game, stone, None, None), 0);
            let mut canceled = false;
            let mut xp = None;
            game.native_break_block([1, 64, 1], ore, &mut canceled, &mut xp);
            assert_eq!(xp, Some(7));
            assert_eq!(drop(game, ore, None, xp), 7);
            add.store(true, Ordering::Relaxed);
            xp = None;
            game.native_break_block([1, 64, 1], ore, &mut canceled, &mut xp);
            assert_eq!(seen.lock().unwrap().as_slice(), &[Some(7), Some(7)]);
            assert_eq!(drop(game, ore, None, xp), 10);
            let mut silk = horizon_client::items::ItemStack::new("minecraft:diamond_pickaxe", 1);
            silk.extra
                .get_or_insert_with(Default::default)
                .enchantments
                .push(("minecraft:silk_touch".into(), 1));
            assert_eq!(drop(game, ore, Some(&silk), None), 0);
            assert_eq!(drop(game, ore, Some(&silk), xp), 10);
            *game.held_mut() = Some(silk.clone());
            xp = None;
            game.native_break_block([1, 64, 1], ore, &mut canceled, &mut xp);
            assert_eq!(seen.lock().unwrap().last(), Some(&Some(0)));
            assert_eq!(xp, Some(3));
            assert_eq!(drop(game, ore, Some(&silk), xp), 3);
            *game.held_mut() = None;
            assert_eq!(drop(game, spawner, Some(&silk), None), 7);
            for (answer, expected) in [(-1, 0), (i32::MAX, 32767), (7, 7)] {
                amount.store(answer, Ordering::Relaxed);
                assert_eq!(drop(game, ore, None, None), expected);
            }
            let chest = registry.default_state("minecraft:chest").unwrap();
            let start = ExplosionStart::from(mod_api::__private::built::ExplosionStart {
                dimension: "minecraft:overworld",
                center: [1.0, 65.0, 1.0],
                radius: 4.0,
                fire: false,
                interaction: mod_api::BlockInteraction::Destroy,
                damage_type: "minecraft:explosion",
                source: None,
                owner: None,
            });
            for x in 1..=3 {
                game.level_set([x, 64, 1], if x == 1 { chest } else { stone })
                    .unwrap();
            }
            let mut contents = vec![None; 27];
            contents[0] = Some(horizon_client::items::ItemStack::new(
                "minecraft:diamond",
                3,
            ));
            game.world.block_entities.insert(
                (1, 64, 1),
                horizon_client::containers::BlockEntity::Chest(contents),
            );
            assert_eq!(drop(game, chest, None, None), 7);
            cancel_blast.store(true, Ordering::Relaxed);
            game.server_explode(start.center, 4.0, BlockInteraction::Destroy, false)
                .unwrap();
            assert_eq!(game.world.block(1, 64, 1), chest);
            assert!(blast.0.lock().unwrap().is_empty());
            cancel_blast.store(false, Ordering::Relaxed);
            game.server_explode(start.center, 4.0, BlockInteraction::Keep, false)
                .unwrap();
            assert!(blast.0.lock().unwrap().is_empty());
            assert_eq!(game.world.block(1, 64, 1), chest);
            game.server_explode_as(
                start.center,
                4.0,
                BlockInteraction::Destroy,
                false,
                "minecraft:explosion",
                Some(([1, 64, 1], f32::MAX)),
            )
            .unwrap();
            assert!(blast.0.lock().unwrap().is_empty());
            assert_eq!(game.world.block(1, 64, 1), chest);
            for x in 2..=3 {
                game.level_set([x, 64, 1], stone).unwrap();
            }
            let chest_handle = game
                .block_entity(game.level("minecraft:overworld").unwrap(), [1, 64, 1])
                .unwrap();
            let mut drops = ExplosionDrops::default();
            for x in 1..=4 {
                game.explosion_hit_block(
                    [x, 64, 1],
                    4.0,
                    BlockInteraction::Destroy,
                    &start,
                    &mut drops,
                )
                .unwrap();
            }
            assert_eq!(blast.0.lock().unwrap().as_slice(), &[[1, 64, 1]]);
            for x in 1..=4 {
                assert_eq!(game.world.block(x, 64, 1), game.world.air);
            }
            assert!(!game.world.block_entities.contains_key(&(1, 64, 1)));
            game.with_level(|_, level, _| {
                assert!(mod_api::LevelHost::block_entity_data(level, chest_handle).is_none());
            });
            for mode in 1..=3 {
                game.level_set([1, 64, 1], chest).unwrap();
                let mut contents = vec![None; 27];
                contents[0] = Some(horizon_client::items::ItemStack::new(
                    "minecraft:diamond",
                    3,
                ));
                game.world.block_entities.insert(
                    (1, 64, 1),
                    horizon_client::containers::BlockEntity::Chest(contents),
                );
                blast.1.store(mode, Ordering::Relaxed);
                let mut drops = ExplosionDrops::default();
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    game.explosion_hit_block(
                        [1, 64, 1],
                        4.0,
                        BlockInteraction::Destroy,
                        &start,
                        &mut drops,
                    )
                    .unwrap();
                }));
                assert_eq!(
                    game.world.block(1, 64, 1),
                    game.world.air,
                    "failed callback must not leave a collected block: mode {mode}"
                );
                assert!(!game.world.block_entities.contains_key(&(1, 64, 1)));
                assert!(
                    result.is_ok(),
                    "part failure must be contained: mode {mode}"
                );
            }
        });
    });
}

/// A block's explosion resistance from a shared answer, counting the asks.
#[derive(Clone, Default)]
struct Resist(Arc<Mutex<Option<f32>>>, Arc<AtomicI32>);
#[mod_api::horizon_part]
impl BlockPart for Resist {
    fn explosion_resistance(
        &self,
        _: &dyn LevelHost,
        event: &mut ExplosionResistanceEvent<'_>,
    ) -> Option<f32> {
        assert_eq!(event.explosion.radius, 4.0);
        self.1.fetch_add(1, Ordering::Relaxed);
        *self.0.lock().unwrap()
    }
}
/// Whether a block drops from an explosion: this answer at x 2, none elsewhere.
#[derive(Clone)]
struct Drops(Option<bool>);
#[mod_api::horizon_part]
impl BlockPart for Drops {
    fn can_drop_from_explosion(
        &self,
        _: &dyn LevelHost,
        event: &mut CanDropFromExplosionEvent<'_>,
    ) -> Option<bool> {
        assert_eq!(event.explosion.center, [1.5, 65.5, 3.5]);
        (event.pos.x == 2).then_some(self.0).flatten()
    }
}
#[derive(Clone, Default)]
struct Answers {
    dirt: Resist,
    obsidian: Resist,
    rail: Resist,
}
#[mod_api::__subscriptions]
impl Hooks for Answers {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        for (name, part) in [
            ("minecraft:dirt", &self.dirt),
            ("minecraft:obsidian", &self.obsidian),
            ("minecraft:rail", &self.rail),
        ] {
            let key = r.block_key(name);
            r.block_part(key, part.clone());
        }
        for (name, drops) in [
            ("minecraft:dirt", false),
            ("minecraft:tnt", true),
            ("minecraft:infested_stone", false),
        ] {
            let key = r.block_key(name);
            r.block_part(key, Drops(Some(drops)));
        }
    }
}

/// Runs `test` in a serving game with the `Answers` parts and an empty chunk at the origin.
fn with_answers(answers: &Answers, test: impl FnOnce(&mut Game<'_>)) {
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "explosion_answers",
        Box::new(answers.clone()),
    )]
    .into();
    mod_platform::testing::with_mods(mods.clone(), || {
        crate::game::api_values::install_parts(&mods).unwrap();
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
            test(game);
        });
    });
}

/// A radius-4 `Destroy` blast at `[1.5, 65.5, 3.5]`, beside the cell `[2, 65, 3]`.
fn blast() -> ExplosionStart<'static> {
    ExplosionStart::from(mod_api::__private::built::ExplosionStart {
        dimension: "minecraft:overworld",
        center: [1.5, 65.5, 3.5],
        radius: 4.0,
        fire: false,
        interaction: mod_api::BlockInteraction::Destroy,
        damage_type: "minecraft:explosion",
        source: None,
        owner: None,
    })
}

#[test]
#[ignore = "needs prepared resources and isolated native mod state"]
fn block_parts_answer_explosion_resistance() {
    let answers = Answers::default();
    with_answers(&answers, |game| {
        let registry = game.pack.assets.registry;
        let [dirt, obsidian, rail] = ["minecraft:dirt", "minecraft:obsidian", "minecraft:rail"]
            .map(|name| registry.default_state(name).unwrap());
        let start = blast();
        let explode = |game: &mut Game<'_>, resists| {
            game.server_explode_as(
                start.center,
                4.0,
                BlockInteraction::Destroy,
                false,
                "minecraft:explosion",
                resists,
            )
            .unwrap();
        };
        let cube = |game: &mut Game<'_>| {
            for x in 2..=4 {
                for y in 64..=66 {
                    for z in 2..=4 {
                        game.world.set(x, y, z, dirt).unwrap();
                    }
                }
            }
        };
        let dirt_left = |game: &Game<'_>| {
            (2..=4)
                .flat_map(|x| (64..=66).flat_map(move |y| (2..=4).map(move |z| (x, y, z))))
                .filter(|&(x, y, z)| game.world.block(x, y, z) == dirt)
                .count()
        };
        // The strongest ray has 4 × 1.3 = 5.2 power, and a step through dirt answering 1200
        // costs (1200 + 0.3) × 0.3 ≈ 360: none of the cube goes.
        *answers.dirt.0.lock().unwrap() = Some(1200.0);
        cube(game);
        explode(game, None);
        assert_eq!(dirt_left(game), 27);
        assert!(answers.dirt.1.load(Ordering::Relaxed) > 0);
        // Unanswered, dirt's own 0.5 lets the blast take the dirt beside it.
        *answers.dirt.0.lock().unwrap() = None;
        explode(game, None);
        assert_eq!(game.world.block(2, 65, 3), game.world.air);
        // A dangerous wither skull's cap still wins: (0.8 + 0.3) × 0.3 = 0.33 a step.
        *answers.dirt.0.lock().unwrap() = Some(1200.0);
        cube(game);
        game.combat.blast_resistance_cap = Some(0.8);
        explode(game, None);
        game.combat.blast_resistance_cap = None;
        assert_eq!(game.world.block(2, 65, 3), game.world.air);
        // A negative or non-finite answer is refused for the block's own value.
        let pos = [2, 65, 3];
        for (answer, expected) in [
            (1200.0, Some(1200.0)),
            (0.0, Some(0.0)),
            (-1.0, None),
            (f32::NAN, None),
            (f32::INFINITY, None),
            (f32::NEG_INFINITY, None),
        ] {
            *answers.dirt.0.lock().unwrap() = Some(answer);
            assert_eq!(
                game.part_explosion_resistance(pos, dirt, &start),
                expected,
                "{answer}"
            );
        }
        *answers.dirt.0.lock().unwrap() = None;
        // Obsidian answering 0: a step costs 0.09 and the weakest ray starts at 2.8.
        game.world.set(2, 65, 3, obsidian).unwrap();
        explode(game, None);
        assert_eq!(game.world.block(2, 65, 3), obsidian, "1200 unanswered");
        *answers.obsidian.0.lock().unwrap() = Some(0.0);
        explode(game, None);
        assert_eq!(game.world.block(2, 65, 3), game.world.air);
        // A respawn anchor's water cell still resists as water (100: 30 a step).
        game.world.set(2, 65, 3, obsidian).unwrap();
        explode(game, Some((pos, 100.0)));
        assert_eq!(game.world.block(2, 65, 3), obsidian);
        // A TNT minecart's rails are spared without asking their parts.
        *answers.rail.0.lock().unwrap() = Some(0.0);
        game.world.set(2, 65, 3, rail).unwrap();
        game.combat.rail_proof_blast = true;
        explode(game, None);
        game.combat.rail_proof_blast = false;
        assert_eq!(game.world.block(2, 65, 3), rail);
        assert_eq!(answers.rail.1.load(Ordering::Relaxed), 0);
    });
}

#[test]
#[ignore = "needs prepared resources and isolated native mod state"]
fn block_parts_answer_drops_from_explosions() {
    with_answers(&Answers::default(), |game| {
        let registry = game.pack.assets.registry;
        let [dirt, tnt, infested] = [
            "minecraft:dirt",
            "minecraft:tnt",
            "minecraft:infested_stone",
        ]
        .map(|name| registry.default_state(name).unwrap());
        let start = blast();
        // Each row: the block at x 2 (its part answers) and x 3 (unanswered).
        let hit = |game: &mut Game<'_>, state, z| {
            let mut drops = ExplosionDrops::default();
            for x in 2..=3 {
                game.world.set(x, 64, z, state).unwrap();
                game.explosion_hit_block(
                    [x, 64, z],
                    4.0,
                    BlockInteraction::Destroy,
                    &start,
                    &mut drops,
                )
                .unwrap();
                assert_eq!(game.world.block(x, 64, z), game.world.air);
            }
            drops.stacks
        };
        // Refused dirt drops nothing; the dirt beside it drops one.
        let stacks = hit(game, dirt, 1);
        let dirt_drops: Vec<_> = stacks
            .iter()
            .filter(|(_, stack)| &*stack.item == "minecraft:dirt")
            .map(|(at, stack)| (*at, stack.count))
            .collect();
        assert_eq!(dirt_drops, [([3, 64, 1], 1)]);
        // TNT answering true drops one TNT item and still primes; unanswered it only primes.
        let primed = game.tnts.len();
        let stacks = hit(game, tnt, 3);
        let tnt_drops: Vec<_> = stacks
            .iter()
            .filter(|(_, stack)| &*stack.item == "minecraft:tnt")
            .map(|(at, stack)| (*at, stack.count))
            .collect();
        assert_eq!(tnt_drops, [([2, 64, 3], 1)]);
        assert_eq!(game.tnts.len(), primed + 2);
        // Refused infested stone lets no silverfish out; the one beside it does.
        let silverfish = |game: &Game<'_>| {
            game.mobs
                .iter()
                .filter(|m| m.kind.id == "minecraft:silverfish")
                .count()
        };
        hit(game, infested, 5);
        assert_eq!(silverfish(game), 1);
    });
}

/// Stone a dangerous wither skull may not destroy, while `proof` is set. It reads the
/// supplied entity's kind and position first, and records them.
#[derive(Clone)]
struct SkullProof {
    proof: Arc<AtomicBool>,
    seen: Arc<Mutex<Vec<(Option<String>, Option<[f64; 3]>)>>>,
}
#[mod_api::horizon_part]
impl BlockPart for SkullProof {
    fn can_entity_destroy(
        &self,
        host: &dyn mod_api::Host,
        event: &mut CanEntityDestroyEvent,
    ) -> Option<bool> {
        let kind = event.entity.kind(host).map(|kind| kind.id(host).to_owned());
        let position = event.entity.position(host);
        let skull = kind.as_deref() == Some("minecraft:wither_skull")
            && position.is_some_and(|p| (1.0..=2.5).contains(&p[0]));
        self.seen.lock().unwrap().push((kind, position));
        (skull && self.proof.load(Ordering::Relaxed)).then_some(false)
    }
}
struct SkullMod(SkullProof);
#[mod_api::__subscriptions]
impl Hooks for SkullMod {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let key = r.block_key("minecraft:stone");
        r.block_part(key, self.0.clone());
    }
}

#[test]
#[ignore = "needs prepared resources and isolated native mod state"]
fn a_block_part_keeps_its_block_from_a_dangerous_skull_cap() {
    let part = SkullProof {
        proof: Arc::new(AtomicBool::new(false)),
        seen: Arc::default(),
    };
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "skull_proof",
        Box::new(SkullMod(part.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods.clone(), || {
        crate::game::api_values::install_parts(&mods).unwrap();
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            // A dangerous skull flying east into a stone block at (2, 64, 0): its impact
            // blasts at radius 1 and keeps it readable while the stone's part is asked.
            let survives = |game: &mut Game<'_>| {
                game.world.set(2, 64, 0, stone).unwrap();
                let mut skull = horizon_client::projectiles::Projectile::new(
                    horizon_client::projectiles::ProjectileKind::WitherSkull { dangerous: true },
                    Vec3::new(1.5, 64.25, 0.5),
                    None,
                );
                skull.velocity = Vec3::new(0.6, 0.0, 0.0);
                game.spawn_mob_projectile(skull);
                for _ in 0..5 {
                    game.tick_projectiles_now().unwrap();
                }
                assert!(game.creatures.projectiles.is_empty(), "the skull hit");
                game.world.block(2, 64, 0) == stone
            };
            // Capped at 0.8, a step through the stone costs (0.8 + 0.3) × 0.3 = 0.33.
            assert!(!survives(game));
            // Answering no for the skull, the stone keeps its 6: (6 + 0.3) × 0.3 = 1.89 a
            // step, more than a ray's 1.3 at most.
            part.proof.store(true, Ordering::Relaxed);
            assert!(survives(game));
            let seen = part.seen.lock().unwrap();
            assert!(!seen.is_empty());
            for (kind, position) in seen.iter() {
                assert_eq!(kind.as_deref(), Some("minecraft:wither_skull"));
                assert!(
                    position.is_some_and(|p| (1.0..=2.5).contains(&p[0])),
                    "{position:?}"
                );
            }
        });
    });
}
