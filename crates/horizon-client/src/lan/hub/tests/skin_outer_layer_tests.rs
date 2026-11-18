//! A player's skin parts reach its own client: once the client's information names
//! them, on every change, and again after a respawn. The clients tracking the
//! player hear them too, including the default they go back to.
use super::*;

/// `ServerEntity.sendDirtyEntityData`'s `sendToTrackingPlayersAndSelf`: a player's
/// own entity is never paired with it (`ChunkMap.TrackedEntity.updatePlayer` skips
/// the entity itself), so the skin parts (`Avatar.DATA_PLAYER_MODE_CUSTOMISATION`)
/// its model shows reach its client as own data alone: once the client's information
/// names them, then every change, and again after a respawn (#17123).
#[test]
fn a_players_skin_parts_reach_its_own_client() {
    fn skin(rx: &std::sync::mpsc::Receiver<crate::lan::Out>) -> Vec<Vec<(u8, i32, Value)>> {
        let mut frames = Vec::new();
        while let Ok(out) = rx.try_recv() {
            let crate::lan::Out::Packet(body) = out else {
                continue;
            };
            let mut r = horizon_protocol::buf::Reader::new(&body);
            let id = r.varint().unwrap();
            if let C::SetEntityData(p) = C::decode(id, r.rest()).unwrap()
                && p.values.iter().any(|(i, _, _)| *i == 16)
            {
                assert_eq!(p.id, 1, "the player's own entity");
                frames.push(p.values);
            }
        }
        frames
    }
    let (mut hub, rx) = hub_with_a_guest();
    hub.sync_player_state(0);
    assert_eq!(
        skin(&rx),
        [vec![(16, serializer::BYTE, Value::Byte(0x7F))]],
        "every layer, the client information's default"
    );
    hub.sync_player_state(0);
    assert!(skin(&rx).is_empty(), "an unchanged mask is not sent again");
    // CAPE (bit 0) and HAT (bit 6) off, as the client's skin customization screen sends.
    hub.guests
        .update(0, |g| g.info.model_customisation = 0x7F & !0x41);
    hub.sync_player_state(0);
    assert_eq!(skin(&rx), [vec![(16, serializer::BYTE, Value::Byte(0x3E))]]);
    // Both ways: a layer put back is a change like any other.
    hub.guests.update(0, |g| g.info.model_customisation = 0x7F);
    hub.sync_player_state(0);
    assert_eq!(
        skin(&rx),
        [vec![(16, serializer::BYTE, Value::Byte(0x7F))]],
        "the layer put back reaches the client"
    );
    hub.sync_player_state(0);
    assert!(skin(&rx).is_empty());
    // A respawn builds the client's new player with zero data (handleRespawn without
    // KEEP_ENTITY_DATA), so the mask it had is gone on its side: the next tick sends it
    // again, or the outer layer stays missing until the skin screen (#17123).
    let mut world = World::new(horizon_world::registry::BlockStateId(0), 0);
    let mut next_entity = 2;
    let border = crate::worldborder::WorldBorder::default();
    let ctx = TickCtx {
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
    hub.respawn(0, [0.5, 64.0, 0.5], Some([0.0; 2]), &ctx);
    assert!(
        skin(&rx).is_empty(),
        "the respawn keeps the mask for the client's fresh player, it does not send it"
    );
    hub.sync_player_state(0);
    assert_eq!(
        skin(&rx),
        [vec![(16, serializer::BYTE, Value::Byte(0x7F))]],
        "the respawned player's client learns its layers again"
    );
    hub.sync_player_state(0);
    assert!(skin(&rx).is_empty());
}

/// `Avatar.DATA_PLAYER_MODE_CUSTOMISATION`'s default is 0, every outer layer
/// hidden, and vanilla's `SynchedEntityData.set` marks a value that comes back to
/// its default dirty like any other: `ServerEntity.sendDirtyEntityData` sends it
/// to the clients that still hold the old mask.
#[test]
fn a_tracking_client_hears_when_skin_parts_go_back_to_their_default() {
    /// The skin parts (index 16) the guest was sent about entity `id`.
    fn parts(
        rx: &std::sync::mpsc::Receiver<crate::lan::Out>,
        id: i32,
    ) -> Vec<Vec<(u8, i32, Value)>> {
        let mut frames = Vec::new();
        while let Ok(out) = rx.try_recv() {
            let crate::lan::Out::Packet(body) = out else {
                continue;
            };
            let mut r = horizon_protocol::buf::Reader::new(&body);
            let pid = r.varint().unwrap();
            if let Ok(C::SetEntityData(p)) = C::decode(pid, r.rest())
                && p.id == id
            {
                let sent: Vec<_> = (p.values.into_iter())
                    .filter(|(index, _, _)| *index == 16)
                    .collect();
                if !sent.is_empty() {
                    frames.push(sent);
                }
            }
        }
        frames
    }
    let (mut hub, _own) = hub_with_a_guest();
    // A second guest beside the first, in the chunk they stand in: it tracks the
    // first player and holds that player's entity data.
    let (tx, rx) = std::sync::mpsc::channel();
    let mut watcher = Guest::new(
        crate::lan::Joined {
            conn: 2,
            uuid: 2,
            name: "Ann".into(),
            properties: Vec::new(),
            info: horizon_protocol::packets::configuration::ClientInformation::new(8),
            brand: "vanilla".into(),
            sender: crate::lan::PlayerSender::new(tx, Arc::default()),
            latency: Arc::new(std::sync::atomic::AtomicI32::new(0)),
            address: "127.0.0.1:25566".parse().unwrap(),
            expected: false,
        },
        2,
        [1.5, 64.0, 0.5],
        8,
    );
    watcher.placed = true;
    watcher.dimension = "minecraft:overworld".into();
    watcher.sent_chunks.insert((0, 0));
    hub.guests.push(watcher);
    let tracked = hub.guests[0].entity_id;
    // The test hub syncs no entity type registry.
    hub.entity_types.insert("minecraft:player".into(), 1);
    let mut world = World::new(horizon_world::registry::BlockStateId(0), 0);
    let mut next_entity = 3;
    let border = crate::worldborder::WorldBorder::default();
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
    hub.track_entities(&mut ctx);
    assert!(hub.guests[1].tracker.is_tracking(tracked));
    assert_eq!(
        parts(&rx, tracked),
        [vec![(16, serializer::BYTE, Value::Byte(0x7F))]],
        "the client that starts tracking the player is sent its skin parts"
    );
    // Every outer layer turned off, as the skin customization screen sends: the
    // base layer alone stays visible.
    hub.guests.update(0, |g| g.info.model_customisation = 0);
    hub.track_entities(&mut ctx);
    assert_eq!(
        parts(&rx, tracked),
        [vec![(16, serializer::BYTE, Value::Byte(0))]],
        "the default goes back to the clients holding the old mask"
    );
    // And up again: a layer turned back on is a change like any other.
    hub.guests.update(0, |g| g.info.model_customisation = 0x7F);
    hub.track_entities(&mut ctx);
    assert_eq!(
        parts(&rx, tracked),
        [vec![(16, serializer::BYTE, Value::Byte(0x7F))]],
        "the layers turned back on reach the tracking client"
    );
}
