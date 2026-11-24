//! A wolf's body armour in a running server: `Wolf.hurtArmor` wears the piece
//! on a hit the armour does not take, and a piece that breaks goes out as the
//! body slot's break event that each client answers with `breakItem`, not as a
//! raw break sound.
use crate::game::Game;
use crate::game::native_machines::tests::server_game;
use horizon_client::mobs::{Damage, DamageKind, Mob, Who};
use horizon_core::geometry::Vec3;

const WOLF: u32 = 9;
const AT: [f64; 3] = [8.5, 64.0, 8.5];

fn game_with_armoured_wolf() -> Game<'static> {
    let mut game = server_game();
    let kind = game.res.mob_types().get("minecraft:wolf").unwrap().clone();
    let mut wolf = Mob::new(WOLF, kind, Vec3::new(AT[0], AT[1], AT[2]), 0.0);
    wolf.health = 100.0;
    wolf.extra.set_body(Some("minecraft:wolf_armor".into()));
    game.mobs.push(wolf);
    game.audio.capture = Some(Vec::new());
    game
}

/// The end of the tick: every mob's outbox reaches `handle_mob_events`.
fn drain(game: &mut Game<'_>) {
    let mut events = std::mem::take(&mut game.mobs[0].outbox);
    game.handle_mob_events_reusable(&mut events).unwrap();
}

#[test]
#[ignore = "daily"]
fn thorns_wear_a_wolfs_body_piece_and_a_break_sends_its_event() {
    let mut game = game_with_armoured_wolf();
    let thorns = Damage {
        kind: DamageKind::Thorns,
        attacker: Some(Who::Player),
        from: None,
        attacker_species: None,
    };
    assert!(
        game.mob_hits_mob(WOLF, 24.0, thorns),
        "the thorns hit landed"
    );
    drain(&mut game);
    let wolf = game.mobs.iter().find(|m| m.id == WOLF).unwrap();
    // `#bypasses_wolf_armor` leaves the hit to the wolf, and
    // `LivingEntity.hurtArmor`'s `doHurtEquipment` wears max(1, 24 / 4) of it.
    assert_eq!(wolf.extra.body_damage(), 6, "max(1, 24 / 4)");
    assert_eq!(wolf.extra.body_id(), Some("minecraft:wolf_armor"));
    assert!(wolf.health < 100.0, "the wolf took the hit itself");

    // 63 points worn and one more absorbed hit: `ItemStack.hurtAndBreak` breaks
    // the piece, and `onEquippedItemBroken` broadcasts the body's event.
    let wolf = game.mobs.iter_mut().find(|m| m.id == WOLF).unwrap();
    wolf.extra.set_body_damage(63);
    wolf.invulnerable = 0;
    wolf.last_hurt = 0.0;
    game.audio.capture.as_mut().unwrap().clear();
    game.lan_entity_events.clear();
    assert!(game.mob_hits_mob(WOLF, 1.0, Damage::of(DamageKind::MobAttack)));
    drain(&mut game);
    let wolf = game.mobs.iter().find(|m| m.id == WOLF).unwrap();
    assert_eq!(wolf.extra.body_id(), None, "the piece broke");
    assert_eq!(wolf.extra.body_damage(), 0);
    assert!(
        game.lan_entity_events
            .iter()
            .any(|&(id, event)| id == WOLF as i32 && event == 65),
        "the body slot's break event: {:?}",
        game.lan_entity_events
    );
    assert!(
        !game
            .audio
            .capture
            .as_ref()
            .unwrap()
            .iter()
            .any(|s| s.event == "minecraft:item.wolf_armor.break"),
        "no raw break sound: the clients play it from the event"
    );
}

#[test]
#[ignore = "daily"]
fn wolf_break_event_reaches_clients_before_the_empty_body_slot() {
    use horizon_client::lan::Out;
    use horizon_protocol::{buf::Reader, packets::play::Clientbound};

    let mut game = game_with_armoured_wolf();
    let rx = crate::game::lan::mob_swing_sync_tests::guest_at(&mut game, AT);
    game.lan
        .as_mut()
        .unwrap()
        .hub
        .update_guest(1, |guest| {
            guest.sent_chunks.insert((0, 0));
        })
        .unwrap();
    game.tick_lan(true);
    rx.try_iter().for_each(drop);
    let wolf = &mut game.mobs[0];
    wolf.extra.set_body_damage(63);
    assert!(game.mob_hits_mob(WOLF, 1.0, Damage::of(DamageKind::MobAttack)));
    drain(&mut game);
    game.tick_lan(true);
    let packets: Vec<_> = rx
        .try_iter()
        .flat_map(|out| match out {
            Out::Packet(body) => vec![body],
            Out::Packets(bodies) => bodies,
            Out::Disconnect(_) => panic!("unexpected disconnect"),
        })
        .filter_map(|body| {
            let mut r = Reader::new(&body);
            let id = r.varint().ok()?;
            Clientbound::decode(id, r.rest()).ok()
        })
        .collect();
    let event = packets
        .iter()
        .position(
            |p| matches!(p, Clientbound::EntityEvent(e) if e.id == WOLF as i32 && e.event == 65),
        )
        .expect("body break event");
    let empty = packets.iter().position(|p| matches!(p, Clientbound::SetEquipment(e) if e.id == WOLF as i32 && e.slots.iter().any(|(slot, stack)| *slot == 6 && stack.is_none()))).expect("empty body slot");
    assert!(
        event < empty,
        "the client needs the old stack for sound and crumbs: {packets:?}"
    );
}
