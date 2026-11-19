//! `ServerGamePacketListenerImpl.handlePlayerPositionChange` spares a player in
//! `LivingEntity.isInPostImpulseGraceTime` (the 40 ticks after a wind charge's
//! push, `ServerPlayer.onExplosionHit`) the moved-wrongly check: a launched or
//! gliding player whose move the server's collision would cut short is not
//! sent back (#15065).
use super::*;

#[test]
fn a_wind_charged_guest_is_not_sent_back_for_moving_wrongly() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/horizon-client")
        .join(horizon_core::VANILLA_VERSION);
    if !root.join("cached-collision.nbt").exists() {
        return; // CI has no vanilla data.
    }
    let assets = crate::assets::Assets::load(&root).unwrap();
    let resources = crate::motion::MotionResources::load(&root, &assets).unwrap();
    let registry = assets.registry;
    let collision = crate::collisions::load(&root, registry).unwrap();
    let air = registry.default_state("minecraft:air").unwrap();
    let stone = registry.default_state("minecraft:stone").unwrap();
    // A two-block stone wall at x = 1 between where the guest stands and
    // where its client says it went; both ends are free.
    let mut world = World::new(air, 0);
    let mut chunk = crate::chunk::Chunk::empty(0, 0);
    chunk.lit = true;
    world.chunks.insert((0, 0), std::sync::Arc::new(chunk));
    world.set(1, 64, 0, stone).unwrap();
    world.set(1, 65, 0, stone).unwrap();
    let border = crate::worldborder::WorldBorder::default();
    let mut next_entity = 2;
    let ctx = TickCtx {
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
    let start = [0.5, 64.0, 0.5];
    let to = [2.5, 64.0, 0.5];
    let moved = |grace: i32| {
        let (mut hub, _rx) = hub_with_a_guest();
        let mut body = crate::motion::PlayerMotion::new(
            &resources,
            horizon_core::geometry::Vec3::new(0.5, 64.0, 0.5),
        )
        .unwrap();
        body.impulse.apply_grace_time(grace);
        hub.guests.update(0, |g| {
            g.position = start;
            g.motion = Some(Box::new(body));
        });
        let mut events = Vec::new();
        hub.position_change(0, to, (0.0, 0.0), 0, Hearing::Heard, &ctx, &mut events);
        let wrongly = events
            .iter()
            .any(|e| matches!(e, HostEvent::Warn(line) if line.contains("moved wrongly")));
        (hub.guests[0].position, wrongly)
    };
    // Without an impulse the wall cuts the move short: moved wrongly, sent back.
    assert_eq!(moved(0), (start, true));
    // In the grace time after a wind charge the move stands.
    assert_eq!(moved(40), (to, false));
}
