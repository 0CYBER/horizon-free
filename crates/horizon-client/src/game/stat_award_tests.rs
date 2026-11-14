//! The statistics `ServerPlayer` awards for what the player does, not the
//! counters a mob or a block posts: a ground jump (`ServerPlayer.jumpFromGround`),
//! a worn-out piece of equipment (`ServerPlayer.onEquippedItemBroken`) and the
//! bed slept in (`ServerPlayer.startSleepInBed`, whose statistic the bed picks:
//! `StrawBedBlock.getSleptInBedStatType`).
use super::super::*;
use crate::game::native_machines::tests::server_game;
use crate::motion::{MotionWorld, PlayerMotion};
use horizon_client::items::ItemStack;
use horizon_world::registry::BlockStateId;

/// `ServerPlayer.jumpFromGround`: a jump off the ground counts as `custom.jump`.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_ground_jump_awards_the_jump_statistic() {
    let mut game = server_game();
    let mut player = PlayerMotion::new(game.motion, Vec3::new(2.5, 64.0, 2.5)).unwrap();
    player.ground.on_ground = true;
    {
        let mut world = MotionWorld {
            entities: &[],
            player: &mut player,
            scene: &mut game.world,
            assets: &game.pack.assets,
            collision_binding: game.collision,
            resources: game.motion,
            health: &game.health_resources,
        };
        world.jump_from_ground().unwrap();
    }
    assert!(player.velocity.y > 0.0, "the jump pushed the player up");
    let events = std::mem::take(&mut player.events);
    game.player = Some(player);
    // The player's turn on the server reads the tick's events (`guest_server_tick`).
    game.record_combat(&events);
    assert_eq!(game.stats.get("custom", "minecraft:jump"), 1);
}

/// `ServerPlayer.onEquippedItemBroken` for a helmet worn out by a hit: the
/// piece leaves the slot and `broken.<item>` counts it.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_worn_out_helmet_awards_the_broken_statistic() {
    let mut game = server_game();
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(2.5, 64.0, 2.5)).unwrap());
    let mut helmet = ItemStack::new("minecraft:leather_helmet", 1);
    helmet.damage = 54;
    game.armor[0] = Some(helmet);
    // `getDamageAfterArmorAbsorb` -> `Player.hurtArmor`: max(1, 4 / 4) points.
    let source = game
        .health_resources
        .damage
        .id("minecraft:generic")
        .unwrap();
    game.hurt_armor(source, 4.0);
    assert!(game.armor[0].is_none(), "the helmet broke off the head");
    assert_eq!(game.stats.get("broken", "minecraft:leather_helmet"), 1);
}

/// A bed's state with its `part`, facing north.
fn bed_state(game: &Game<'_>, kind: &str, part: &str) -> BlockStateId {
    let registry = game.pack.assets.registry;
    let mut state = registry
        .state(registry.default_state(kind).unwrap())
        .unwrap()
        .clone();
    state.properties.insert("part".into(), part.into());
    registry.id(&state).unwrap()
}

/// Both halves of a bed of `kind` at `x`, facing north: the foot at z+1, the
/// head at z. Answers the head.
fn place_bed(game: &mut Game<'_>, x: i32, kind: &str) -> [i32; 3] {
    let foot = bed_state(game, kind, "foot");
    let head = bed_state(game, kind, "head");
    game.world.set(x, 64, 3, foot).unwrap();
    game.world.set(x, 64, 2, head).unwrap();
    [x, 64, 2]
}

/// `ServerPlayer.startSleepInBed` awards the bed's own statistic: a straw bed
/// counts as `sleep_in_straw_bed` (`StrawBedBlock.getSleptInBedStatType`), and
/// an ordinary bed still as `sleep_in_bed`.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn sleeping_in_a_straw_bed_awards_its_own_statistic() {
    let mut game = server_game();
    // `Level.isDarkOutside`: midnight, so a bed may be slept in.
    game.world.day_time = 18000;
    let straw = place_bed(&mut game, 2, "minecraft:straw_bed");
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(2.5, 64.0, 3.5)).unwrap());
    game.try_sleep(straw);
    assert_eq!(
        game.sleeping,
        Some(straw),
        "the player lay in the straw bed"
    );
    assert_eq!(game.stats.get("custom", "minecraft:sleep_in_straw_bed"), 1);
    assert_eq!(game.stats.get("custom", "minecraft:sleep_in_bed"), 0);
    game.wake_up();
    // An ordinary bed keeps the plain statistic.
    let red = place_bed(&mut game, 4, "minecraft:red_bed");
    game.player.as_mut().unwrap().position = Vec3::new(4.5, 64.0, 3.5);
    game.try_sleep(red);
    assert_eq!(game.sleeping, Some(red), "the player lay in the red bed");
    assert_eq!(game.stats.get("custom", "minecraft:sleep_in_bed"), 1);
    assert_eq!(game.stats.get("custom", "minecraft:sleep_in_straw_bed"), 1);
}
