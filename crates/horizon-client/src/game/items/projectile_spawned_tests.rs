//! `Projectile.applyOnProjectileSpawned` through `Game::spawn_projectile` and
//! the mod host, with enchantments from a world datapack. Expected values come
//! from vanilla's formulas run on copies of the randoms involved.
#![cfg(test)]
use crate::game::native_entity_refs::tests::with_game;
use horizon_client::{
    items::ItemStack,
    mobs::{Mob, Who},
    projectiles::{Projectile, ProjectileKind},
};
use horizon_core::{
    geometry::Vec3,
    random::{EntityRandom, LegacyRandom, RandomSource},
};
use mod_api::Identifier;
use serde_json::{Value, json};

/// A world whose datapack defines `spawn_test:<name>` for each
/// `(name, projectile_spawned effects)`.
fn with_enchantments(defs: &[(&str, Value)], test: impl FnOnce(&mut super::Game<'_>)) {
    with_game(|game| {
        let root = std::env::temp_dir().join(format!(
            "horizon-projectile-spawned-{}-{}",
            std::process::id(),
            defs.iter()
                .map(|(name, _)| *name)
                .collect::<Vec<_>>()
                .join("-")
        ));
        let pack = root.join("datapacks/spawned");
        let definitions = pack.join("data/spawn_test/enchantment");
        std::fs::create_dir_all(&definitions).unwrap();
        std::fs::write(
            pack.join("pack.mcmeta"),
            r#"{"pack":{"description":"projectile_spawned fixture"}}"#,
        )
        .unwrap();
        std::fs::write(
            root.join("datapacks.json"),
            r#"{"enabled":["file/spawned"],"disabled":[]}"#,
        )
        .unwrap();
        for (name, effects) in defs {
            let definition = json!({
                "description": "fixture", "supported_items": "#minecraft:enchantable/bow",
                "weight": 1, "max_level": 3,
                "min_cost": {"base": 1, "per_level_above_first": 0},
                "max_cost": {"base": 1, "per_level_above_first": 0},
                "anvil_cost": 1, "slots": ["mainhand"],
                "effects": {"minecraft:projectile_spawned": effects},
            });
            std::fs::write(
                definitions.join(format!("{name}.json")),
                serde_json::to_vec(&definition).unwrap(),
            )
            .unwrap();
        }
        game.world_root = root.clone();
        let stack = game.reload_datapacks(false, &[]).unwrap();
        let resources = game.options.resources.clone();
        game.world_tables = game
            .stage_world_tables(&stack, false, &resources)
            .unwrap()
            .0;
        for (name, _) in defs {
            assert!(
                game.stations()
                    .enchantments
                    .get(&format!("spawn_test:{name}"))
                    .is_some(),
                "{name} did not load"
            );
        }
        test(game);
        let _ = std::fs::remove_dir_all(root);
    });
}

fn enchanted(item: &str, count: u32, levels: &[(&str, u32)]) -> ItemStack {
    let mut stack = ItemStack::new(item, count);
    stack.data_mut().enchantments = levels
        .iter()
        .map(|(id, level)| ((*id).into(), *level))
        .collect();
    stack
}

fn sound(name: &str) -> Value {
    json!({"type": "minecraft:play_sound", "sound": name,
        "volume": {"type": "minecraft:uniform", "min_inclusive": 0.5, "max_exclusive": 1.0},
        "pitch": {"type": "minecraft:uniform", "min_inclusive": 1.0, "max_exclusive": 2.0}})
}

fn ignite(seconds: f32) -> Value {
    json!({"type": "minecraft:ignite", "duration": seconds})
}

/// The sounds played since capture began, as (event, volume, pitch).
fn played(game: &mut super::Game<'_>) -> Vec<(String, f32, f32)> {
    game.audio
        .capture
        .replace(Vec::new())
        .unwrap_or_default()
        .into_iter()
        .map(|s| {
            assert_eq!(s.source, mod_api::SoundSource::Neutral as usize);
            (s.event, s.volume, s.pitch)
        })
        .collect()
}

/// Each effect's requirements are tested with the level random just before
/// it applies, and only a matching effect draws from the projectile's random:
/// a coin-flip sound draws twice when it plays and not at all when it
/// doesn't, and the next effect's draws follow on.
#[test]
#[ignore = "daily"]
fn requirements_draw_the_level_random_before_each_effect() {
    let coin = json!({"condition": "minecraft:random_chance", "chance": 0.5});
    let effects = json!([
        {"requirements": coin, "effect": sound("minecraft:entity.arrow.shoot")},
        {"effect": sound("minecraft:entity.snowball.throw")},
        // Arrows only: the projectile is the context's `this`.
        {"requirements": {"condition": "minecraft:entity_properties", "entity": "this",
            "predicate": {"type": "minecraft:arrow"}}, "effect": ignite(3.0)},
    ]);
    with_enchantments(&[("coin", effects)], |game| {
        let stack = enchanted("minecraft:snowball", 1, &[("spawn_test:coin", 1)]);
        let mut both = false;
        let mut neither = false;
        for seed in 0..32_i64 {
            let mut level = game.world.random.clone();
            // LootItemRandomChanceCondition: nextFloat() < chance.
            let flipped = level.next_float() < 0.5;
            let mut own = LegacyRandom::new(seed);
            // UniformFloat.sample: nextFloat() * (max - min) + min, volume first.
            let mut uniform = |min: f32, max: f32| own.next_float() * (max - min) + min;
            let mut expected = Vec::new();
            if flipped {
                let (v, p) = (uniform(0.5, 1.0), uniform(1.0, 2.0));
                expected.push(("minecraft:entity.arrow.shoot".to_owned(), v, p));
            }
            let (v, p) = (uniform(0.5, 1.0), uniform(1.0, 2.0));
            expected.push(("minecraft:entity.snowball.throw".to_owned(), v, p));
            let after = own.next_long();
            game.audio.capture = Some(Vec::new());
            let shot = Projectile::new(ProjectileKind::Snowball, Vec3::new(0.5, 70.0, 0.5), None)
                .with_entity_random(EntityRandom::own(LegacyRandom::new(seed)));
            let id = game.spawn_projectile(shot, &stack);
            assert_eq!(played(game), expected, "seed {seed}");
            let p = game
                .creatures
                .projectiles
                .iter()
                .find(|p| p.id == id)
                .unwrap();
            assert_eq!(p.random.0.clone().next_long(), after, "seed {seed}");
            // The entity_properties requirement draws nothing and fails.
            assert_eq!(p.fire_ticks, 0);
            assert_eq!(game.world.random.next_long(), level.next_long());
            (both, neither) = (both || flipped, neither || !flipped);
        }
        assert!(both && neither, "the coin landed both ways");
    });
}

/// The stack's effects run before the weapon's, in the stack's enchantment
/// order and each enchantment's effect order; an arrow fired from its own
/// item runs that item's effects once.
#[test]
#[ignore = "daily"]
fn stack_then_weapon_in_component_order() {
    let defs = [
        (
            "first",
            json!([{"effect": {"type": "minecraft:all_of", "effects": [
                sound("minecraft:block.note_block.bell"),
                // On a projectile: nothing, and no draws.
                {"type": "minecraft:apply_mob_effect", "to_apply": "minecraft:speed",
                    "min_duration": 1, "max_duration": 9, "min_amplifier": 0, "max_amplifier": 3},
                ignite(2.0),
            ]}}]),
        ),
        (
            "second",
            json!([{"effect": sound("minecraft:block.note_block.chime")}]),
        ),
        (
            "weapon",
            json!([
                {"effect": sound("minecraft:block.note_block.flute")},
                {"effect": ignite(5.0)},
            ]),
        ),
    ];
    with_enchantments(&defs, |game| {
        let ammo = enchanted(
            "minecraft:arrow",
            1,
            &[("spawn_test:first", 1), ("spawn_test:second", 1)],
        );
        let bow = enchanted("minecraft:bow", 1, &[("spawn_test:weapon", 1)]);
        let arrow = |weapon: &ItemStack| {
            let mut p = Projectile::new(
                ProjectileKind::Arrow {
                    base_damage: 2.0,
                    critical: false,
                    effect: None,
                },
                Vec3::new(0.5, 70.0, 0.5),
                Some(Who::Player),
            )
            .with_entity_random(EntityRandom::own(LegacyRandom::new(7)));
            let a = p.arrow.as_mut().unwrap();
            a.set_item(&ammo);
            a.fired_from_weapon = Some(weapon.clone());
            p
        };
        let names = |sounds: Vec<(String, f32, f32)>| -> Vec<String> {
            sounds.into_iter().map(|(name, _, _)| name).collect()
        };
        let order: Vec<String> = ammo
            .enchantments()
            .iter()
            .map(|(id, _)| match &**id {
                "spawn_test:first" => "minecraft:block.note_block.bell",
                _ => "minecraft:block.note_block.chime",
            })
            .chain(["minecraft:block.note_block.flute"])
            .map(str::to_owned)
            .collect();
        game.audio.capture = Some(Vec::new());
        let id = game.spawn_projectile(arrow(&bow), &ammo);
        assert_eq!(names(played(game)), order);
        let p = game
            .creatures
            .projectiles
            .iter()
            .find(|p| p.id == id)
            .unwrap();
        // igniteForTicks keeps the longer fire: the weapon's 5 s.
        assert_eq!(p.arrow.as_ref().unwrap().fire_ticks, 100);
        // The weapon is the ammo's own item: its effects run once.
        let id = game.spawn_projectile(arrow(&ammo), &ammo);
        assert_eq!(names(played(game)).len(), 2);
        let p = game
            .creatures
            .projectiles
            .iter()
            .find(|p| p.id == id)
            .unwrap();
        assert_eq!(p.arrow.as_ref().unwrap().fire_ticks, 40);
    });
}

/// The mod host's owner launch applies the supplied stack's effects once,
/// whatever its count, leaves the stack alone, and refuses a stack with an
/// effect the engine does not apply yet before drawing anything.
#[test]
#[ignore = "daily"]
fn owner_launch_applies_the_supplied_stacks_effects() {
    let defs = [
        ("hot", json!([{"effect": ignite(4.0)}])),
        (
            "flags",
            json!([{"requirements": {"condition":"minecraft:entity_properties", "entity":"this", "predicate":{"minecraft:flags":{"is_on_fire":true}}}, "effect": ignite(4.0)}]),
        ),
        (
            "bad_clamp",
            json!([{"effect":{"type":"minecraft:ignite", "duration":{"type":"minecraft:clamped", "value":1,"min":2,"max":0}}}]),
        ),
        (
            "boom",
            json!([{"effect": {"type": "minecraft:explode", "radius": 1,
            "block_interaction": "none", "small_particle": {"type": "minecraft:explosion"},
            "large_particle": {"type": "minecraft:explosion_emitter"},
            "sound": "minecraft:entity.generic.explode"}}]),
        ),
    ];
    with_enchantments(&defs, |game| {
        let kind = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        game.mobs
            .push(Mob::new(21, kind, Vec3::new(0.0, 64.0, 0.0), 0.0));
        let source = game.entity(21).unwrap();
        let owner = game.living(source).unwrap();
        let snowball = Identifier::parse("minecraft:snowball").unwrap();
        let stack = enchanted("minecraft:snowball", 16, &[("spawn_test:hot", 2)]);
        let shot = game
            .spawn_projectile_from_rotation(owner, &snowball, &stack, 0.0, 1.5, 1.0)
            .unwrap();
        let p = game
            .creatures
            .projectiles
            .iter()
            .find(|p| p.id == shot.entity().id())
            .unwrap();
        // Ignite(4): 80 ticks, once for the stack, not per item in it.
        assert_eq!(p.fire_ticks, 80);
        assert_eq!(p.item.as_ref().unwrap().count, 1);
        assert_eq!(stack.count, 16);
        let boom = enchanted("minecraft:snowball", 1, &[("spawn_test:boom", 1)]);
        let count = game.creatures.projectiles.len();
        let mut before = game.world.random.clone();
        assert!(
            game.spawn_projectile_from_rotation(owner, &snowball, &boom, 0.0, 1.5, 1.0)
                .is_none()
        );
        assert_eq!(game.creatures.projectiles.len(), count);
        assert_eq!(game.world.random.next_long(), before.next_long());
        for enchantment in ["spawn_test:flags", "spawn_test:bad_clamp"] {
            let invalid = enchanted("minecraft:snowball", 1, &[(enchantment, 1)]);
            let mut before = game.world.random.clone();
            assert!(
                game.spawn_projectile_from_rotation(owner, &snowball, &invalid, 0.0, 1.5, 1.0)
                    .is_none()
            );
            assert_eq!(game.creatures.projectiles.len(), count);
            assert_eq!(game.world.random.next_long(), before.next_long());
        }
    });
}
