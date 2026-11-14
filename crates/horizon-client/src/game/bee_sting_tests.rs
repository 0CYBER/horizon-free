//! `Bee.doHurtTarget`: the stinger, the calm-down and the sting sound follow
//! only a hit that landed. A creative player, a raised shield and an
//! invulnerable mob refuse it, and the bee keeps its stinger.
use super::*;
use horizon_client::items::ItemStack;
use horizon_client::mobs::{Mob, Who, ai::do_hurt_target, bee, goals::Cx};
use horizon_client::motion::PlayerMotion;
use horizon_core::geometry::Vec3;
use horizon_core::random::LegacyRandom;
use horizon_world::food::Difficulty;

fn spawn(game: &mut Game<'_>, id: u32, kind: &str, position: Vec3) -> u32 {
    let kind = game.res.mob_types().get(kind).unwrap().clone();
    game.mobs.push(Mob::new(id, kind, position, 0.0));
    id
}

/// One `Bee.doHurtTarget` from the bee, then the game's event handling.
fn sting(game: &mut Game<'_>, bee_id: u32, who: Who) {
    let mut mobs = std::mem::take(&mut game.mobs);
    let i = mobs.iter().position(|m| m.id == bee_id).unwrap();
    let mut random = LegacyRandom::new(0);
    let mut events = Vec::new();
    {
        let w = game.mob_world_base(&[]);
        let mut cx = Cx {
            w: &w,
            lr: &mut random,
            events: &mut events,
        };
        do_hurt_target(&mut mobs[i], who, &mut cx);
    }
    game.mobs = mobs;
    game.handle_mob_events(events).unwrap();
}

fn sting_sounds(game: &mut Game<'_>) -> Vec<(f32, [f64; 3])> {
    game.audio
        .capture
        .take()
        .unwrap_or_default()
        .into_iter()
        .filter(|s| s.event == "minecraft:entity.bee.sting")
        .map(|s| (s.pitch, s.position))
        .collect()
}
/// An angry bee that stings a victim which refuses the hit keeps its stinger,
/// its anger and its sound.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_refused_sting_leaves_the_bee_alone() {
    let mut game = super::super::native_machines::tests::server_game();
    game.world.health_rules.difficulty = Difficulty::Normal;
    // `Invulnerable:1b`: `hurtServer` reports no hurt.
    game.audio.capture = Some(Vec::new());
    let bee = spawn(&mut game, 41, "minecraft:bee", Vec3::new(1.5, 64.0, 0.5));
    let pig = spawn(&mut game, 42, "minecraft:pig", Vec3::new(0.5, 64.0, 0.5));
    game.mobs
        .iter_mut()
        .find(|m| m.id == pig)
        .unwrap()
        .invulnerable_flag = true;
    let bee_mob = game.mobs.iter_mut().find(|m| m.id == bee).unwrap();
    let b = bee_mob.extra.bee.get_or_insert_with(Default::default);
    b.angry_at = Some(Who::Mob(pig));
    b.anger_end = 400;
    sting(&mut game, bee, Who::Mob(pig));
    let bee_mob = game.mobs.iter().find(|m| m.id == bee).unwrap();
    assert!(!bee::has_stung(bee_mob), "invulnerable mob");
    assert_eq!(bee::get(bee_mob).unwrap().angry_at, Some(Who::Mob(pig)));
    assert!(sting_sounds(&mut game).is_empty());

    game.mobs.clear();
    game.audio.capture = Some(Vec::new());
    game.creative = true;
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
    let bee = spawn(&mut game, 43, "minecraft:bee", Vec3::new(1.5, 64.0, 0.5));
    sting(&mut game, bee, Who::Player);
    let bee_mob = game.mobs.iter().find(|m| m.id == bee).unwrap();
    assert!(!bee::has_stung(bee_mob));
    assert!(sting_sounds(&mut game).is_empty());

    game.mobs.clear();
    game.audio.capture = Some(Vec::new());
    game.creative = false;
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
    game.yaw = 0.0;
    game.offhand = Some(ItemStack::new("minecraft:shield", 1));
    // `BlocksAttacks.blockDelayTicks` of the shield: 0.25s.
    game.using = Some((super::super::shield::OFFHAND_SLOT, 5));
    let bee = spawn(&mut game, 44, "minecraft:bee", Vec3::new(0.5, 64.0, 2.5));
    sting(&mut game, bee, Who::Player);
    let bee_mob = game.mobs.iter().find(|m| m.id == bee).unwrap();
    assert!(!bee::has_stung(bee_mob));
    assert!(sting_sounds(&mut game).is_empty());
}

/// A sting that lands sets the flag, calms the bee, plays the sound at its
/// position and poisons the victim on normal and hard.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_landed_sting_sets_the_stinger_and_poisons_on_normal_and_hard() {
    let mut game = super::super::native_machines::tests::server_game();
    for (difficulty, poison_ticks) in [(Difficulty::Normal, 200), (Difficulty::Hard, 360)] {
        game.mobs.clear();
        game.world.health_rules.difficulty = difficulty;
        game.audio.capture = Some(Vec::new());
        let bee = spawn(&mut game, 41, "minecraft:bee", Vec3::new(1.5, 64.0, 0.5));
        let pig = spawn(&mut game, 42, "minecraft:pig", Vec3::new(0.5, 64.0, 0.5));
        let bee_mob = game.mobs.iter_mut().find(|m| m.id == bee).unwrap();
        let b = bee_mob.extra.bee.get_or_insert_with(Default::default);
        b.angry_at = Some(Who::Mob(pig));
        b.anger_end = 400;
        sting(&mut game, bee, Who::Mob(pig));
        let bee_mob = game.mobs.iter().find(|m| m.id == bee).unwrap();
        assert!(bee::has_stung(bee_mob), "{difficulty:?}");
        assert_eq!(bee::get(bee_mob).unwrap().angry_at, None, "{difficulty:?}");
        let pig_mob = game.mobs.iter().find(|m| m.id == pig).unwrap();
        assert!(pig_mob.health < 10.0, "{difficulty:?}");
        let poison = pig_mob
            .extra
            .effects
            .iter()
            .find(|(effect, _, _)| *effect == "minecraft:poison");
        assert_eq!(
            poison.map(|(_, ticks, amplifier)| (*ticks, *amplifier)),
            Some((poison_ticks, 0)),
            "{difficulty:?}"
        );
        assert_eq!(
            sting_sounds(&mut game),
            [(1.0, [1.5, 64.0, 0.5])],
            "{difficulty:?}"
        );
    }
}
