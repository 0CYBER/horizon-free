//! A player's skin parts reach its own client: once the client's information names
//! them, on every change, and again after a respawn.
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
