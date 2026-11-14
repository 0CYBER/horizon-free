//! A conversion is `Mob.convertTo`: `ConversionType.convertCommon` carries the
//! old mob's name, silence, no-AI flag and effects to the new one, and the
//! caller's `AfterConversion` step runs on it (`Pig.thunderHit`,
//! `MushroomCow.shear`).
use super::super::*;
use horizon_client::chunk::Chunk;
use horizon_client::mobs::{MobEvent, Species};
use horizon_client::motion::PlayerMotion;
use horizon_core::{geometry::Vec3, position::ChunkPos, random::RandomSource};
use std::sync::Arc;

fn game() -> Game<'static> {
    let root = std::path::PathBuf::from(
        std::env::var_os("HORIZON_TEST_RESOURCES")
            .expect("set HORIZON_TEST_RESOURCES to prepared resources"),
    );
    let settings = Settings::default();
    let loaded = load_resources(
        root.clone(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        true,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = execution_role::ExecutionRole::IntegratedServer;
    let res: &'static _ = Box::leak(Box::new(loaded.resources));
    let mut game = Game::new(
        res,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&root),
        None,
    )
    .unwrap();
    game.build_registries();
    game.set_allow_commands(true);
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
    game.screen = Screen::Playing;
    game.selected = 0;
    game.set_mode_id(0);
    game
}

/// `Pig.thunderHit`: the pig becomes a zombified piglin that keeps its name,
/// silence, no-AI flag and baby age (#7510), holds the golden sword or spear
/// the conversion rolls off the pig's own random, and is made persistent. It
/// is not nauseous: only `AbstractPiglin.finishConversion` and
/// `Hoglin.finishConversion` add that.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_struck_pig_leaves_a_zombified_piglin_that_keeps_its_state() {
    let mut game = game();
    // Easy: `Pig.thunderHit` converts outside peaceful, and the strike itself
    // only starts fires on normal and hard.
    game.ui.difficulty = 1;
    assert!(game.run_command(
        "summon minecraft:pig 8.5 64 8.5 {CustomName:{text:\"Bolt\"},NoAI:1b,Silent:1b}"
    ));
    // A baby pig stays a baby through the conversion (#7510).
    let index = game
        .mobs
        .iter()
        .position(|m| m.species() == Species::Pig)
        .expect("the pig was summoned");
    game.mobs[index].extra.age = -24000;
    let pig = game.mobs[index].clone();
    assert!(pig.is_baby(), "the summoned pig is a baby");
    assert!(!pig.persistent, "a summoned pig is not persistent");
    // `Pig.thunderHit` hands `ZombifiedPiglin.populateDefaultEquipmentSlots`
    // the pig's own random (`this.getRandom()`): one `nextInt(20)`, a golden
    // spear on 0 and a golden sword otherwise.
    let mut random = pig.random.clone();
    let expected = if random.next_int_bound(20).unwrap_or(1) == 0 {
        "minecraft:golden_spear"
    } else {
        "minecraft:golden_sword"
    };

    game.spawn_bolt([8.5, 64.0, 8.5], false);
    game.tick_bolts();

    assert!(
        game.mobs
            .iter()
            .all(|m| m.species() != Species::Pig || m.removed),
        "the pig is gone"
    );
    let zombified = game
        .mobs
        .iter()
        .find(|m| m.species() == Species::ZombifiedPiglin)
        .expect("the pig became a zombified piglin");
    assert_eq!(zombified.custom_name.as_deref(), Some("Bolt"));
    assert!(zombified.is_baby(), "the baby pig stays a baby (#7510)");
    assert!(zombified.silent, "the silence is carried over");
    assert!(zombified.no_ai, "the no-AI flag is carried over");
    assert_eq!(
        zombified.extra.main_hand.as_deref(),
        Some(expected),
        "the gear the conversion rolls"
    );
    assert!(zombified.persistent, "Pig.thunderHit makes it persistent");
    assert!(
        !zombified
            .extra
            .effects
            .iter()
            .any(|e| e.0 == "minecraft:nausea"),
        "a pig struck by lightning is not nauseous"
    );
}

/// `MushroomCow.shear`: the cow that takes the mooshroom's place keeps its
/// name, silence, no-AI flag and effects, and keeps its persistence as
/// `convertCommon` copies it (a summoned mooshroom leaves a cow that is not
/// persistent).
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_sheared_mooshroom_leaves_a_cow_that_keeps_its_state() {
    let mut game = game();
    assert!(game.run_command(
        "summon minecraft:mooshroom 2.5 64 2.5 {CustomName:{text:\"Bessie\"},NoAI:1b,Silent:1b}"
    ));
    let mooshroom = game
        .mobs
        .iter()
        .find(|m| m.species() == Species::Mooshroom)
        .expect("the mooshroom was summoned")
        .id;
    game.inventory[0] = Some(ItemStack::new("minecraft:shears", 1));
    assert!(
        game.interact_mob(Some(InteractTarget {
            id: mooshroom as i32,
            location: [0.0, 0.0, 0.0],
        }))
        .unwrap(),
        "the shears are used"
    );
    let cow = game
        .mobs
        .iter()
        .find(|m| m.species() == Species::Cow)
        .expect("the mooshroom became a cow");
    assert_eq!(cow.custom_name.as_deref(), Some("Bessie"));
    assert!(cow.silent, "the silence is carried over");
    assert!(cow.no_ai, "the no-AI flag is carried over");
    assert!(!cow.persistent, "the summoned mooshroom was not persistent");
    assert_eq!(cow.position, Vec3::new(2.5, 64.0, 2.5), "in its place");
}

/// `Villager.thunderHit`: `single(this, false, false)`, then
/// `finalizeSpawn(CONVERSION)` and `setPersistenceRequired`, so the witch keeps
/// the villager's name and silence.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_struck_villager_leaves_a_witch_that_keeps_its_state() {
    let mut game = game();
    game.ui.difficulty = 1;
    assert!(game.run_command(
        "summon minecraft:villager 2.5 64 2.5 {CustomName:{text:\"Elder\"},Silent:1b}"
    ));
    game.spawn_bolt([2.5, 64.0, 2.5], false);
    game.tick_bolts();
    let witch = game
        .mobs
        .iter()
        .find(|m| m.species() == Species::Witch)
        .expect("the villager became a witch");
    assert_eq!(witch.custom_name.as_deref(), Some("Elder"));
    assert!(witch.silent, "the silence is carried over");
    assert!(witch.persistent, "Villager.thunderHit makes it persistent");
    // `Raider.finalizeSpawn`: a witch made by a conversion can join a raid.
    assert!(
        horizon_client::mobs::raider::state(witch).is_some_and(|r| r.can_join_raid),
        "the conversion's finalizeSpawn lets it join a raid"
    );
}

/// `AbstractPiglin.finishConversion`: a piglin that zombifies leaves a
/// zombified piglin nauseous for 10 seconds, the step its conversion event
/// carries (a pig struck by lightning gets none).
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_zombifying_piglin_leaves_a_nauseous_zombified_piglin() {
    let mut game = game();
    assert!(game.run_command("summon minecraft:piglin 2.5 64 2.5 {}"));
    let index = game
        .mobs
        .iter()
        .position(|m| m.species() == Species::Piglin)
        .expect("the piglin was summoned");
    let from = game.mobs[index].clone();
    game.mobs[index].removed = true;
    game.handle_mob_events(vec![MobEvent::Convert {
        from: Box::new(from),
        kind: "minecraft:zombified_piglin",
        params: horizon_client::mobs::conversion::Params::KEEP_ALL,
        after: horizon_client::mobs::conversion::After::Nausea,
    }])
    .unwrap();
    let zombified = game
        .mobs
        .iter()
        .find(|m| m.species() == Species::ZombifiedPiglin)
        .expect("the piglin became a zombified piglin");
    let nausea = zombified
        .extra
        .effects
        .iter()
        .find(|e| e.0 == "minecraft:nausea");
    assert_eq!(
        nausea.map(|e| e.1),
        Some(200),
        "nauseous for 200 ticks, as `MobEffectInstance(NAUSEA, 200, 0)`"
    );
}
