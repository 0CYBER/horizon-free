//! Sound delivery, seed order and emitter lifetime through the real hub.
use super::*;
use horizon_core::random::RandomSource;

fn next_sent_packet(rx: &std::sync::mpsc::Receiver<crate::lan::Out>) -> C {
    let crate::lan::Out::Packet(bytes) = rx.try_recv().expect("a sound packet") else {
        panic!("expected a packet");
    };
    let mut reader = horizon_protocol::buf::Reader::new(&bytes);
    let id = reader.varint().unwrap();
    C::decode(id, reader.rest()).unwrap()
}

#[test]
fn a_trident_joining_after_tracking_is_paired_before_its_throw_sound() {
    let mut sync = SyncData::empty();
    sync.builtin.registries.insert(
        "minecraft:entity_type".into(),
        horizon_protocol::registry::Ids::from_names(vec!["minecraft:trident".into()]),
    );
    let (mut hub, rx) = hub_with_sync(sync);
    hub.sync_attachments = true;
    hub.guests.update(0, |g| g.sent_chunks.insert((0, 0)));
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
        spawn_dimension: "minecraft:overworld",
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
    while rx.try_recv().is_ok() {}
    let trident = EntityState {
        id: 37,
        uuid: 37,
        kind: "minecraft:trident".into(),
        position: [0.5, 65.5, 0.5],
        velocity: [0.25, 0.0, 0.0],
        acceleration_power: None,
        ..EntityState::default()
    };
    hub.note_joined(std::slice::from_ref(&trident));
    hub.captured_sound(
        &crate::audio::CapturedSound {
            event: "minecraft:item.trident.throw".into(),
            position: trident.position,
            volume: 1.0,
            pitch: 1.0,
            source: 7,
            entity: Some(37),
            except_entity: None,
        },
        None,
    );
    assert!(
        matches!(next_sent_packet(&rx), C::AddEntity(p) if p.id == 37),
        "ChunkMap.addEntity pairs a late spawn before the throw sound"
    );
    let mut heard = false;
    while let Ok(out) = rx.try_recv() {
        let crate::lan::Out::Packet(bytes) = out else {
            panic!("unexpected batch")
        };
        let mut reader = horizon_protocol::buf::Reader::new(&bytes);
        let id = reader.varint().unwrap();
        if matches!(C::decode(id, reader.rest()).unwrap(), C::SoundEntity(p) if p.id == 37) {
            heard = true;
        }
    }
    assert!(heard);
    hub.collect_tracking();
    assert_eq!(hub.native_tracking.len(), 1);
    assert_eq!(hub.native_tracking[0].events.len(), 1);
    assert!(hub.native_tracking[0].events[0].start);
    assert_eq!(hub.native_tracking[0].events[0].entity, 37);
    assert!(
        hub.attachment_snapshots
            .iter()
            .any(|s| s.player == hub.guests[0].uuid && s.holder == Holder::Entity(37))
    );
    hub.native_tracking.clear();
    hub.attachment_snapshots.clear();
    hub.note_joined(std::slice::from_ref(&trident));
    hub.collect_tracking();
    assert!(hub.native_tracking.is_empty());
    assert!(hub.attachment_snapshots.is_empty());
    assert!(rx.try_recv().is_err());
}

#[test]
fn a_captured_sound_reaches_the_client_before_its_emitter_is_removed() {
    let mut sync = SyncData::empty();
    sync.builtin.registries.insert(
        "minecraft:entity_type".into(),
        horizon_protocol::registry::Ids::from_names(vec!["minecraft:mooshroom".into()]),
    );
    let (mut hub, rx) = hub_with_sync(sync);
    hub.guests.update(0, |g| g.sent_chunks.insert((0, 0)));
    let mooshroom = EntityState {
        id: 37,
        uuid: 37,
        kind: "minecraft:mooshroom".into(),
        position: [0.5, 64.0, 0.5],
        ..EntityState::default()
    };
    let mut world = World::new(horizon_world::registry::BlockStateId(0), 0);
    let border = crate::worldborder::WorldBorder::default();
    let mut next_entity = 38;
    let mut ctx = TickCtx {
        sea_level: 63,
        world: &mut world,
        entities: std::slice::from_ref(&mooshroom),
        next_entity: &mut next_entity,
        spawn: mooshroom.position,
        spawn_angle: [0.0; 2],
        spawn_dimension: "minecraft:overworld",
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
    assert!(hub.guests[0].tracker.is_tracking(37));
    while rx.try_recv().is_ok() {}
    // This is the current game's end-of-tick order: tracking sees the
    // discarded mooshroom, then lan_flush_effects drains the level sounds.
    ctx.entities = &[];
    let sound = crate::audio::CapturedSound {
        event: "minecraft:entity.mooshroom.shear".into(),
        position: mooshroom.position,
        volume: 1.0,
        pitch: 1.0,
        source: 7,
        entity: Some(37),
        except_entity: None,
    };
    let mut sounds = vec![sound];
    hub.prepare_captured_sounds(&mut sounds, ctx.entities);
    hub.track_entities(&mut ctx);
    for sound in sounds {
        hub.captured_sound(&sound, None);
    }
    hub.finish_captured_sounds();
    let mut order = Vec::new();
    while let Ok(crate::lan::Out::Packet(bytes)) = rx.try_recv() {
        let mut reader = horizon_protocol::buf::Reader::new(&bytes);
        let id = reader.varint().unwrap();
        match C::decode(id, reader.rest()).unwrap() {
            C::SoundEntity(p) if p.id == 37 => order.push("sound"),
            C::RemoveEntities(p) if p.ids.contains(&37) => order.push("remove"),
            _ => {}
        }
    }
    assert_eq!(
        order,
        ["sound", "remove"],
        "the native shear sound precedes discard"
    );
}

#[test]
fn a_new_pair_does_not_reorder_sounds_or_remove_an_old_emitter_early() {
    for new_first in [false, true] {
        for batching in [false, true] {
            let mut sync = SyncData::empty();
            sync.builtin.registries.insert(
                "minecraft:entity_type".into(),
                horizon_protocol::registry::Ids::from_names(vec![
                    "minecraft:mooshroom".into(),
                    "minecraft:happy_ghast".into(),
                ]),
            );
            let (mut hub, rx) = hub_with_sync(sync);
            hub.guests.update(0, |g| g.sent_chunks.insert((0, 0)));
            let old = EntityState {
                id: 37,
                uuid: 37,
                kind: "minecraft:mooshroom".into(),
                position: [0.5, 64.0, 0.5],
                ..EntityState::default()
            };
            let other = EntityState {
                id: 39,
                uuid: 39,
                ..old.clone()
            };
            let fresh = EntityState {
                id: 38,
                uuid: 38,
                kind: "minecraft:happy_ghast".into(),
                ..old.clone()
            };
            let mut world = World::new(horizon_world::registry::BlockStateId(0), 0);
            let border = crate::worldborder::WorldBorder::default();
            let mut next_entity = 40;
            let previous = [old.clone(), other];
            let current = [fresh];
            let mut ctx = TickCtx {
                sea_level: 63,
                world: &mut world,
                entities: &previous,
                next_entity: &mut next_entity,
                spawn: old.position,
                spawn_angle: [0.0; 2],
                spawn_dimension: "minecraft:overworld",
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
            assert!(hub.guests[0].tracker.is_tracking(37));
            assert!(hub.guests[0].tracker.is_tracking(39));
            while rx.try_recv().is_ok() {}
            *hub.sound_seeds.lock().unwrap() = LegacyRandom::new(0);
            let mut random = LegacyRandom::new(0);
            let old_sound = crate::audio::CapturedSound {
                event: "minecraft:entity.mooshroom.shear".into(),
                position: old.position,
                volume: 1.0,
                pitch: 1.0,
                source: 7,
                entity: Some(37),
                except_entity: None,
            };
            let fresh_sound = crate::audio::CapturedSound {
                event: "minecraft:entity.ghastling.spawn".into(),
                source: 4,
                entity: Some(38),
                ..old_sound.clone()
            };
            let mut sounds = if new_first {
                vec![fresh_sound, old_sound]
            } else {
                vec![old_sound, fresh_sound]
            };
            let holding = batching.then(crate::lan::Holding::start);
            ctx.entities = &current;
            hub.prepare_captured_sounds(&mut sounds, ctx.entities);
            assert_eq!(sounds.len(), if new_first { 2 } else { 1 });
            hub.track_entities(&mut ctx);
            for sound in sounds {
                hub.captured_sound(&sound, None);
            }
            hub.finish_captured_sounds();
            drop(holding);
            hub.guests[0].sender.flush();
            let mut packets = Vec::new();
            while let Ok(message) = rx.try_recv() {
                match message {
                    crate::lan::Out::Packet(body) => packets.push(body),
                    crate::lan::Out::Packets(batch) => packets.extend(batch),
                    _ => panic!("unexpected disconnect"),
                }
            }
            let mut pair = None;
            let mut old_remove = None;
            let mut other_remove = None;
            let mut heard = Vec::new();
            for (index, body) in packets.iter().enumerate() {
                let mut reader = horizon_protocol::buf::Reader::new(body);
                let id = reader.varint().unwrap();
                match C::decode(id, reader.rest()).unwrap() {
                    C::AddEntity(p) if p.id == 38 => {
                        assert!(pair.replace(index).is_none(), "one pairing")
                    }
                    C::SoundEntity(p) => heard.push((p.id, p.seed, index)),
                    C::RemoveEntities(p) => {
                        if p.ids.contains(&37) {
                            assert!(
                                old_remove.replace(index).is_none(),
                                "one removal of the emitter"
                            );
                        }
                        if p.ids.contains(&39) {
                            assert!(
                                other_remove.replace(index).is_none(),
                                "one unrelated removal"
                            );
                        }
                    }
                    _ => {}
                }
            }
            assert_eq!(
                heard.iter().map(|h| h.0).collect::<Vec<_>>(),
                if new_first {
                    vec![38, 37]
                } else {
                    vec![37, 38]
                }
            );
            assert_eq!(
                heard.iter().map(|h| h.1).collect::<Vec<_>>(),
                vec![random.next_long(), random.next_long()]
            );
            assert_eq!(hub.next_sound_seed(), random.next_long());
            let old_sound = heard.iter().find(|h| h.0 == 37).unwrap().2;
            let new_sound = heard.iter().find(|h| h.0 == 38).unwrap().2;
            assert!(
                pair.unwrap() < new_sound,
                "the new emitter must be paired first"
            );
            assert!(
                old_sound < old_remove.unwrap(),
                "the old emitter must survive its sound"
            );
            assert!(
                other_remove.unwrap() < pair.unwrap(),
                "unrelated removals keep their normal place"
            );
            if !new_first {
                assert!(old_sound < other_remove.unwrap());
            }
        }
    }
}

#[test]
fn direct_removals_split_the_sound_emitters_and_do_not_leak_into_the_next_batch() {
    for targeted in [false, true] {
        let (hub, rx) = hub_with_a_guest();
        let sound = crate::audio::CapturedSound {
            event: "fixture:bound".into(),
            position: [0.5, 64.0, 0.5],
            volume: 1.0,
            pitch: 1.0,
            source: 6,
            entity: Some(38),
            except_entity: None,
        };
        let mut sounds = vec![
            sound.clone(),
            crate::audio::CapturedSound {
                entity: Some(37),
                ..sound
            },
        ];
        let fresh = EntityState {
            id: 38,
            ..EntityState::default()
        };
        hub.prepare_captured_sounds(&mut sounds, std::slice::from_ref(&fresh));
        let removal = C::RemoveEntities(play::RemoveEntities { ids: vec![37, 99] });
        if targeted {
            hub.send_to(1, &removal);
        } else {
            hub.broadcast(&removal);
        }
        let C::RemoveEntities(packet) = next_sent_packet(&rx) else {
            panic!("expected unrelated removal")
        };
        assert_eq!(packet.ids, vec![99]);
        assert!(rx.try_recv().is_err(), "the sound emitter is held");
        for sound in sounds {
            hub.captured_sound(&sound, None);
        }
        assert!(matches!(next_sent_packet(&rx),C::SoundEntity(p) if p.id==38));
        assert!(matches!(next_sent_packet(&rx),C::SoundEntity(p) if p.id==37));
        hub.finish_captured_sounds();
        let C::RemoveEntities(packet) = next_sent_packet(&rx) else {
            panic!("expected held removal")
        };
        assert_eq!(packet.ids, vec![37]);
        hub.finish_captured_sounds();
        assert!(rx.try_recv().is_err(), "the held removal is not sent twice");
        hub.broadcast(&C::RemoveEntities(play::RemoveEntities { ids: vec![37] }));
        let C::RemoveEntities(packet) = next_sent_packet(&rx) else {
            panic!("expected immediate removal after the batch")
        };
        assert_eq!(packet.ids, vec![37]);
    }
}

#[test]
fn captured_entity_sound_routes_by_player_dimension_range_and_exclusion() {
    let (mut hub, rx) = hub_with_a_guest();
    *hub.sound_seeds.lock().unwrap() = LegacyRandom::new(0);
    let mut random = LegacyRandom::new(0);
    let mut sound = crate::audio::CapturedSound {
        event: "fixture:bound".into(),
        position: [0.0; 3],
        volume: 1.0,
        pitch: 1.2,
        source: 6,
        entity: Some(37),
        except_entity: None,
    };
    hub.guests.update(0, |g| g.position = [0.0; 3]);
    // The server broadcasts by distance even if this player does not track
    // the emitter; the receiving client decides whether it knows that id.
    hub.captured_sound(&sound, None);
    let C::SoundEntity(packet) = next_sent_packet(&rx) else {
        panic!("expected entity sound")
    };
    assert_eq!(
        (packet.id, packet.source, packet.volume, packet.pitch),
        (37, 6, 1.0, 1.2)
    );
    assert_eq!(packet.seed, random.next_long());
    assert_eq!(packet.sound, (-1, Some(("fixture:bound".into(), None))));

    sound.except_entity = Some(hub.guests[0].entity_id as u32);
    hub.captured_sound(&sound, None);
    random.next_long();
    assert!(rx.try_recv().is_err());
    sound.except_entity = None;
    hub.captured_sound(&sound, Some(hub.guests[0].conn));
    random.next_long();
    assert!(rx.try_recv().is_err());

    {
        let value = [16.0, 0.0, 0.0];
        hub.guests.update(0, |g| g.position = value);
    }
    hub.captured_sound(&sound, None);
    random.next_long();
    assert!(rx.try_recv().is_err(), "the radius itself is excluded");
    {
        let value = [15.99, 0.0, 0.0];
        hub.guests.update(0, |g| g.position = value);
    }
    hub.captured_sound(&sound, None);
    let C::SoundEntity(packet) = next_sent_packet(&rx) else {
        panic!("expected entity sound")
    };
    assert_eq!(
        packet.seed,
        random.next_long(),
        "rejected broadcasts still draw their seeds"
    );

    {
        let value = "minecraft:the_nether".into();
        hub.guests.update(0, |g| g.dimension = value);
    }
    hub.captured_sound(&sound, None);
    random.next_long();
    assert!(rx.try_recv().is_err());
    {
        let value = "minecraft:overworld".into();
        hub.guests.update(0, |g| g.dimension = value);
    }
    {
        let value = false;
        hub.guests.update(0, |g| g.placed = value);
    }
    hub.captured_sound(&sound, None);
    random.next_long();
    assert!(rx.try_recv().is_err());
    assert_eq!(hub.next_sound_seed(), random.next_long());
}

#[test]
fn captured_and_direct_sounds_share_the_server_seed_stream_and_synced_registry() {
    let mut sync = SyncData::empty();
    sync.builtin.registries.insert(
        "minecraft:sound_event".into(),
        horizon_protocol::registry::Ids::from_names(vec!["fixture:bound".into()]),
    );
    let (hub, rx) = hub_with_sync(sync);
    *hub.sound_seeds.lock().unwrap() = LegacyRandom::new(0);
    let mut random = LegacyRandom::new(0);
    let sound = crate::audio::CapturedSound {
        event: "fixture:bound".into(),
        position: [0.5, 64.0, 0.5],
        volume: 1.0,
        pitch: 1.0,
        source: 6,
        entity: Some(37),
        except_entity: None,
    };
    hub.sound("fixture:bound", sound.position, 1.0, 1.0, 6, None);
    let C::Sound(packet) = next_sent_packet(&rx) else {
        panic!("expected positional sound")
    };
    assert_eq!(packet.seed, random.next_long());
    hub.captured_sound(&sound, None);
    let C::SoundEntity(packet) = next_sent_packet(&rx) else {
        panic!("expected entity sound")
    };
    assert_eq!(packet.sound, (0, None));
    assert_eq!(packet.seed, random.next_long());
    hub.sound("fixture:bound", sound.position, 1.0, 1.0, 6, None);
    let C::Sound(packet) = next_sent_packet(&rx) else {
        panic!("expected positional sound")
    };
    assert_eq!(packet.seed, random.next_long());
}

/// The global level event packet a lone guest at `at` in `dimension` gets for
/// an End portal opening at `[10, 64, -3]` in the overworld.
fn global_event_sent_to(at: [f64; 3], dimension: &str) -> play::LevelEvent {
    global_event_sent_while(at, dimension, "minecraft:overworld")
}

/// `global_event_sent_to` with level `in_place` in place when the event is sent.
fn global_event_sent_while(at: [f64; 3], dimension: &str, in_place: &str) -> play::LevelEvent {
    let (mut hub, rx) = hub_with_a_guest();
    hub.config.dimension = in_place.into();
    hub.guests.update(0, |g| {
        g.position = at;
        g.dimension = dimension.into();
    });
    while rx.try_recv().is_ok() {}
    hub.global_level_event("minecraft:overworld", 1038, [10, 64, -3], 0);
    let C::LevelEvent(packet) = next_sent_packet(&rx) else {
        panic!("expected a level event")
    };
    assert!(rx.try_recv().is_err(), "one packet per player");
    packet
}

/// `ServerLevel.globalLevelEvent`: a player in the level within 32 blocks of
/// the event's centre gets the event's own position, with the global flag.
#[test]
fn a_global_level_event_reaches_a_near_player_at_its_position() {
    assert_eq!(
        global_event_sent_to([30.5, 64.0, -3.5], "minecraft:overworld"),
        play::LevelEvent {
            kind: 1038,
            pos: [10, 64, -3],
            data: 0,
            global: true,
        }
    );
}

/// A player in the level further away gets the block 32 blocks from them
/// towards the event's centre: from (70.5, 144.5, -2.5) the centre is
/// (-60, -80, 0) away, 100 blocks, so 32 of them land at (51.3, 118.9, -2.5).
#[test]
fn a_global_level_event_reaches_a_far_player_32_blocks_towards_it() {
    assert_eq!(
        global_event_sent_to([70.5, 144.5, -2.5], "minecraft:overworld").pos,
        [51, 118, -3]
    );
}

/// A player in another level gets the block they stand in, whatever the
/// distance: the event is heard as if it came from them.
#[test]
fn a_global_level_event_reaches_a_player_in_another_level_at_their_own_block() {
    let packet = global_event_sent_to([-7.3, 40.0, 12.9], "minecraft:the_nether");
    assert_eq!(packet.pos, [-8, 40, 12]);
    assert!(packet.global);
}

/// An event queued during the overworld's pass and sent during the Nether's
/// keeps its own level: an overworld player near it gets its position, not
/// their own block as a player of another level would.
#[test]
fn a_global_level_event_keeps_its_level_when_another_level_is_in_place() {
    assert_eq!(
        global_event_sent_while(
            [30.5, 64.0, -3.5],
            "minecraft:overworld",
            "minecraft:the_nether"
        )
        .pos,
        [10, 64, -3]
    );
}
