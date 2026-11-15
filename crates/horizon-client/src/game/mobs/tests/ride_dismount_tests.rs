//! `/ride`: mounting keeps the rider's view (`ServerPlayer.startRiding`
//! teleports with `Relative.ROTATION`), a mob that boards a mount after its
//! player takes the seat after the player (`Entity.addPassenger` puts a player
//! first), and getting off follows the vehicle's own
//! `getDismountLocationForPassenger`.
use super::*;
use horizon_client::mobs::riders;

/// A stone floor whose top is y 64, and the blocks listed above it.
struct Ground {
    solid: Vec<[i32; 3]>,
}

impl Ground {
    fn with(blocks: &[[i32; 3]]) -> Self {
        Self {
            solid: blocks.to_vec(),
        }
    }
    fn solid(&self, p: [i32; 3]) -> bool {
        p[1] == 63 || self.solid.contains(&p)
    }
}

impl DismountWorld for Ground {
    fn lava(&self, _: [i32; 3]) -> bool {
        false
    }
    /// `BlockGetter.getBlockFloorHeight`: a full block is its own top, the
    /// block above one is the floor at 0, and air over air has none.
    fn floor_height(&self, p: [i32; 3]) -> f64 {
        if self.solid(p) {
            1.0
        } else if self.solid([p[0], p[1] - 1, p[2]]) {
            0.0
        } else {
            f64::NEG_INFINITY
        }
    }
    fn can_dismount_to(&self, bounds: Aabb) -> bool {
        let (lo, hi) = (bounds.min_position(), bounds.max_position());
        let cells = |a: f64, b: f64| a.floor() as i32..=(b - 1.0e-7).floor() as i32;
        !cells(lo.x, hi.x)
            .any(|x| cells(lo.y, hi.y).any(|y| cells(lo.z, hi.z).any(|z| self.solid([x, y, z]))))
    }
}

/// A mount of `species`, `width` wide and `height` tall, standing at `at`.
fn mount(species: Species, width: f32, height: f64, at: [f64; 3], yaw: f32) -> Mount {
    let half = f64::from(width) / 2.0;
    Mount {
        species,
        position: Vec3::new(at[0], at[1], at[2]),
        bounds: Aabb::new(
            Vec3::new(at[0] - half, at[1], at[2] - half),
            Vec3::new(at[0] + half, at[1] + height, at[2] + half),
        ),
        width,
        yaw,
    }
}

/// A right-handed player facing `yaw`.
fn player(yaw: f32) -> Rider {
    Rider {
        yaw,
        width: 0.6,
        poses: DISMOUNT_POSES
            .iter()
            .map(|&pose| (pose, pose.dimensions(1.0)))
            .collect(),
        main_arm_left: false,
    }
}

/// `Entity.getDismountLocationForPassenger`: a zombie has no rule of its own,
/// so the rider comes out on top of its box, not beside it.
#[test]
fn getting_off_a_zombie_lands_on_its_top() {
    let zombie = mount(Species::Zombie, 0.6, 1.95, [0.5, 64.0, 0.5], 0.0);
    let (at, pose) = dismount_location(&zombie, &player(0.0), &Ground::with(&[]));
    assert_eq!(at, Vec3::new(0.5, 64.0 + 1.95, 0.5));
    assert_eq!(pose, None);
}

/// `AbstractHorse.getDismountLocationForPassenger`: a block in the main hand's
/// column sends the rider up onto it, not round to the other side.
#[test]
fn getting_off_a_horse_steps_up_on_its_main_hand_side() {
    let width = 1.4f32;
    let horse = mount(Species::Horse, width, 1.6, [0.5, 64.0, 0.5], 0.0);
    let ground = Ground::with(&[[-1, 64, 0]]);
    let (at, pose) = dismount_location(&horse, &player(0.0), &ground);
    // Right hand of a south-facing horse: one block west, at the edge of the
    // square its box spans.
    let step = escape_vector(f64::from(width), f64::from(0.6f32), 90.0);
    assert!((step.x + 1.000_005).abs() < 1.0e-6, "{step:?}");
    assert!(step.z.abs() < 1.0e-6, "{step:?}");
    assert_eq!(at, Vec3::new(0.5 + step.x, 65.0, 0.5 + step.z));
    assert_eq!(pose, Some(Pose::Standing));
}

/// `Animal.getDismountLocationForPassenger`: the blocks round the pig in its
/// own facing's order, at its own height.
#[test]
fn getting_off_a_pig_steps_to_the_block_beside_its_facing() {
    let pig = mount(Species::Pig, 0.9, 0.9, [0.5, 64.0, 0.5], 0.0);
    let ground = Ground::with(&[[-1, 64, 0]]);
    let (at, pose) = dismount_location(&pig, &player(0.0), &ground);
    // Facing south: the right-hand block is west and taken, so the left one.
    assert_eq!(at, Vec3::new(1.5, 64.0, 0.5));
    assert_eq!(pose, Some(Pose::Standing));
}

/// The local player standing at 8.5 64 8.5, able to run commands.
fn scene(game: &mut Game<'_>) {
    game.build_registries();
    game.set_allow_commands(true);
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(8.5, 64.0, 8.5)).unwrap());
}

const GHAST: &str = "0000ca3e-0000-0000-0000-000000000007";

/// Summons the happy ghast of `GHAST` at 8.5 64 8.5 and returns its id.
fn ghast(game: &mut Game<'_>) -> u32 {
    assert!(game.run_command(
        "summon minecraft:happy_ghast 8.5 64 8.5 {Tame:1b,NoAI:1b,UUID:[I;51774,0,0,7]}"
    ));
    let m = game
        .mobs
        .iter_mut()
        .find(|m| m.species() == Species::HappyGhast)
        .expect("the happy ghast");
    m.yaw = 0.0;
    m.id
}

/// `ServerPlayer.startRiding`: the teleport is `Relative.ROTATION` with no
/// delta, so the player's view is the one it had.
#[test]
fn a_ride_mount_keeps_the_players_view() {
    // No prepared resources (CI): nothing to run in.
    let mut resources = None;
    let Some(mut game) = super::tests::server_game(&mut resources) else {
        return;
    };
    scene(&mut game);
    let id = ghast(&mut game);
    game.yaw = 90.0;
    assert!(game.run_command(&format!("ride @s mount {GHAST}")));
    assert_eq!(game.riding, Some(id));
    assert_eq!(game.yaw, 90.0, "the mount's yaw is not the player's");
}

/// `HappyGhast.doPlayerRide` is `player.startRiding(this)` and nothing else,
/// so right-clicking a harnessed one keeps the view too (unlike a horse's
/// `AbstractHorse.doPlayerRide`, which turns the rider first).
#[test]
fn a_click_on_a_harnessed_ghast_keeps_the_players_view() {
    let mut resources = None;
    let Some(mut game) = super::tests::server_game(&mut resources) else {
        return;
    };
    scene(&mut game);
    let id = ghast(&mut game);
    let index = game
        .mobs
        .iter()
        .position(|m| m.id == id)
        .expect("the ghast");
    game.mobs[index]
        .extra
        .set_body(Some("minecraft:white_harness".into()));
    game.yaw = 90.0;
    assert!(game.interact_mount(index, "minecraft:air"));
    assert_eq!(game.riding, Some(id));
    assert_eq!(game.yaw, 90.0, "the ghast's yaw is not the player's");
}

/// `Entity.addPassenger`: the player is the first passenger, so the pig that
/// boards after it sits in the next seat.
#[test]
fn a_mob_that_boards_a_happy_ghast_after_its_player_takes_the_next_seat() {
    let mut resources = None;
    let Some(mut game) = super::tests::server_game(&mut resources) else {
        return;
    };
    scene(&mut game);
    let id = ghast(&mut game);
    assert!(game.run_command(&format!("ride @s mount {GHAST}")));
    assert!(game.run_command("summon minecraft:pig 4.5 64 8.5 {NoAI:1b}"));
    let pig = game
        .mobs
        .iter()
        .position(|m| m.species() == Species::Pig)
        .expect("the pig");
    game.mobs[pig].extra.mount = Some(id);
    game.position_riders();
    let (at, seat) = (game.mobs[pig].position, riders::happy_ghast_attachment(1));
    let ghast = game.mobs.iter().find(|m| m.id == id).unwrap().position;
    assert_eq!(at.x, ghast.x + seat[0], "the left seat, not the player's");
    assert_eq!(at.z, ghast.z + seat[2]);
    assert_ne!(at.z, ghast.z + riders::happy_ghast_attachment(0)[2]);
}
