//! A gliding guest's motion reaches the players tracking it.
use super::*;

/// The velocities of the SetEntityMotion packets about `id` a guest was sent.
fn sent_motions(rx: &std::sync::mpsc::Receiver<crate::lan::Out>, id: i32) -> Vec<[f64; 3]> {
    let mut sent = Vec::new();
    while let Ok(out) = rx.try_recv() {
        let crate::lan::Out::Packet(body) = out else {
            continue;
        };
        let mut r = horizon_protocol::buf::Reader::new(&body);
        let packet_id = r.varint().unwrap();
        if let Ok(C::SetEntityMotion(p)) = C::decode(packet_id, r.rest())
            && p.id == id
        {
            sent.push(p.velocity);
        }
    }
    sent
}

/// ServerEntity.sendChanges: a gliding player's delta movement (its server
/// copy's, `entity.getDeltaMovement()`) goes to the players tracking it
/// (`sendToTrackingPlayers`), not to the player itself.
#[test]
fn a_gliding_guests_motion_goes_to_the_others() {
    let root = std::env::var_os("HORIZON_TEST_RESOURCES")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../.local/horizon-client")
                .join(horizon_core::VANILLA_VERSION)
        });
    if !root.join("movement-properties.nbt").exists() {
        return;
    }
    let assets = crate::assets::Assets::load(&root).unwrap();
    let resources = crate::motion::MotionResources::load(&root, &assets).unwrap();
    let body =
        crate::motion::PlayerMotion::new(&resources, horizon_core::geometry::Vec3::ZERO).unwrap();
    let mut sync = SyncData::empty();
    sync.builtin.registries.insert(
        "minecraft:entity_type".into(),
        horizon_protocol::registry::Ids::from_names(vec!["minecraft:player".into()]),
    );
    let (mut hub, glider_rx) = hub_with_sync(sync);
    hub.guests.update(0, |g| {
        g.sent_chunks.insert((0, 0));
        g.motion = Some(Box::new(body));
        g.fall_flying = true;
    });
    let (tx, watcher_rx) = std::sync::mpsc::channel();
    let mut ada = Guest::new(
        crate::lan::Joined {
            conn: 2,
            uuid: 2,
            name: "Ada".into(),
            properties: Vec::new(),
            info: horizon_protocol::packets::configuration::ClientInformation::new(8),
            brand: "vanilla".into(),
            sender: crate::lan::PlayerSender::new(tx, Arc::default()),
            latency: Arc::new(std::sync::atomic::AtomicI32::new(0)),
            address: "127.0.0.1:25566".parse().unwrap(),
            expected: false,
        },
        2,
        [0.5, 64.0, 0.5],
        8,
    );
    ada.placed = true;
    ada.dimension = "minecraft:overworld".into();
    ada.sent_chunks.insert((0, 0));
    hub.guests.push(ada);
    let mut world = World::new(horizon_world::registry::BlockStateId(0), 0);
    let border = crate::worldborder::WorldBorder::default();
    let mut next_entity = 38;
    let mut ctx = TickCtx {
        sea_level: 63,
        world: &mut world,
        entities: &[],
        next_entity: &mut next_entity,
        spawn: [0.5, 64.0, 0.5],
        spawn_angle: [0.0; 2],
        difficulty: 2,
        difficulty_locked: false,
        clock_rate: 1.0,
        tick_rate: 20.0,
        tick_frozen: false,
        tick_steps: Default::default(),
        collision: None,
        border: &border,
        streaming: None,
    };
    // Paired with the glide's motion at rest; then it speeds up.
    hub.track_entities(&mut ctx);
    assert!(hub.guests[1].tracker.is_tracking(1));
    let v = horizon_core::geometry::Vec3::new(1.2, -0.3, 0.4);
    hub.guests
        .update(0, |g| g.motion.as_mut().unwrap().velocity = v);
    for _ in 0..3 {
        hub.ticks += 1;
        hub.track_entities(&mut ctx);
    }
    // Once, as the wire's LpVec3 rounds it.
    let sent = sent_motions(&watcher_rx, 1);
    assert_eq!(sent.len(), 1, "{sent:?}");
    let wanted = [1.2, -0.3, 0.4];
    assert!(
        (0..3).all(|k| (sent[0][k] - wanted[k]).abs() < 1e-3),
        "{sent:?}"
    );
    assert!(sent_motions(&glider_rx, 1).is_empty());
}
