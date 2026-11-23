//! The anticheat's `ground` check on a claim that carries no new position (a status-only move): it
//! is not judged while the server has the player at rest, and judged like any other claim while the
//! server has the player falling, since accepting it would end the fall.
use super::pending_move_tests::{InboxItem, standing, turned, walking};
use super::*;
use horizon_core::geometry::{Axis, VoxelShape};
use horizon_core::nbt::Tag;
use horizon_world::{
    block_collision::BlockCollisionRegistry,
    collision::CachedCollisionRegistry,
    registry::{BlockRegistry, BlockStateId},
};
use std::sync::Arc;

fn shape_tag(shape: &VoxelShape) -> Tag {
    let sizes = shape.sizes();
    let mut bits = Vec::new();
    for z in 0..sizes[2] {
        for y in 0..sizes[1] {
            for x in 0..sizes[0] {
                bits.push(u8::from(shape.is_full(x as i32, y as i32, z as i32)));
            }
        }
    }
    let (min, max) = shape.discrete_bounds();
    let mut fields = vec![
        ("occupied", Tag::ByteArray(bits)),
        ("cube", Tag::Byte(i8::from(shape.is_cube()))),
        (
            "cube_axes",
            Tag::Byte(
                shape
                    .cube_coordinate_axes()
                    .iter()
                    .enumerate()
                    .fold(0, |mask, (i, on)| mask | (i8::from(*on) << i)),
            ),
        ),
        ("discrete_min", Tag::IntArray(min.to_vec())),
        ("discrete_max", Tag::IntArray(max.to_vec())),
    ];
    for (axis, name) in [(Axis::X, "x"), (Axis::Y, "y"), (Axis::Z, "z")] {
        fields.push((
            name,
            Tag::LongArray(
                shape
                    .coordinates(axis)
                    .iter()
                    .map(|v| v.to_bits() as i64)
                    .collect(),
            ),
        ));
    }
    Tag::compound(fields)
}

/// A block registry of air alone, and its (empty) collision table.
fn air_only() -> (BlockRegistry, Tag) {
    let states = vec![Tag::compound([("id", Tag::String("minecraft:air".into()))])];
    let registry = BlockRegistry::from_nbt(&Tag::compound([
        ("DataVersion", Tag::Int(horizon_world::DATA_VERSION)),
        ("states", Tag::List(states.clone())),
        ("defaults", Tag::compound([("minecraft:air", Tag::Int(0))])),
    ]))
    .unwrap();
    let table = Tag::compound([
        ("DataVersion", Tag::Int(horizon_world::DATA_VERSION)),
        ("states", Tag::List(states)),
        ("shapes", Tag::List(vec![shape_tag(&VoxelShape::empty())])),
        ("cached_shapes", Tag::IntArray(vec![0])),
    ]);
    (registry, table)
}

/// A status-only move that claims the ground.
fn on_the_ground() -> S {
    S::MovePlayerStatus(play::MovePlayerStatus { flags: 1 })
}

/// Guest 0 of `hub` sends `claims` after the packets already in its inbox, in a world of air around
/// `(0, 64, 0)`: the `ground` level it ends with.
fn claim_the_ground(hub: &mut Hub, claims: Vec<S>) -> f32 {
    let (registry, table) = air_only();
    let cached = CachedCollisionRegistry::from_nbt(&table, &registry).unwrap();
    let collision = BlockCollisionRegistry::new(Arc::new(cached)).unwrap();
    let mut world = World::new(BlockStateId(0), 0);
    for x in -1..=0 {
        for z in -1..=0 {
            world
                .chunks
                .insert((x, z), Arc::new(crate::chunk::Chunk::empty(x, z)));
        }
    }
    hub.guests.update(0, |g| {
        g.inbox
            .extend(claims.into_iter().map(InboxItem::uncredited))
    });
    let (mut next_entity, border) = (2, crate::worldborder::WorldBorder::default());
    let mut ctx = TickCtx {
        sea_level: 63,
        world: &mut world,
        entities: &[],
        next_entity: &mut next_entity,
        spawn: [0.5, 64.0, 0.5],
        spawn_angle: [0.0; 2],
        spawn_dimension: "minecraft:overworld",
        difficulty: 2,
        difficulty_locked: false,
        clock_rate: 1.0,
        tick_rate: 20.0,
        tick_frozen: false,
        tick_steps: Default::default(),
        collision: Some(&collision),
        border: &border,
        streaming: None,
    };
    let _ = hub.drain_packets(&mut ctx);
    let config = &hub.anticheat.as_deref().unwrap().config;
    hub.guests[0]
        .anticheat
        .vl(config, horizon_anticheat::Check::Ground, hub.ticks)
}

/// The anticheat's defaults, writing no log and no strikes file.
fn quiet() -> horizon_anticheat::Config {
    horizon_anticheat::Config {
        log: String::new(),
        strikes: String::new(),
        ..horizon_anticheat::Config::default()
    }
}

#[test]
fn a_ground_claim_without_a_position_is_judged_while_the_server_has_the_player_falling() {
    // In mid-air, half a block into a fall: the claim is flagged and dropped, the fall goes on.
    let fell = S::MovePlayerPos(play::MovePlayerPos {
        x: 0.0,
        y: 63.5,
        z: 0.0,
        flags: 0,
    });
    let mut hub = standing(false, vec![fell]);
    hub.set_anticheat(quiet());
    // The same claim again at the same spot, with a turn of the head or without: dropped, but
    // flagged only once.
    let claims = vec![on_the_ground(), turned(30.0), on_the_ground(), turned(60.0)];
    assert_eq!(claim_the_ground(&mut hub, claims), 1.0, "flagged once");
    let g = &hub.guests[0];
    assert_eq!(g.position, [0.0, 63.5, 0.0]);
    assert!(!g.on_ground, "the claims were dropped");

    // Standing still where the server has the player at rest: on the ground, or not falling (a
    // move across): not judged.
    for (on_ground, inbox) in [(true, vec![]), (false, vec![walking(0.5)])] {
        let mut hub = standing(false, inbox);
        hub.guests.update(0, |g| g.on_ground = on_ground);
        hub.set_anticheat(quiet());
        let level = claim_the_ground(&mut hub, vec![on_the_ground()]);
        assert_eq!(level, 0.0, "on the ground: {on_ground}");
        assert!(hub.guests[0].on_ground, "the claim stands");
    }
}
