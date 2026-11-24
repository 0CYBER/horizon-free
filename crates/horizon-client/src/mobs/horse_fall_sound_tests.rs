//! `ClientLevel.playSeededSound` plays a level sound only when the entity it
//! excludes is the local player; a mob's `Entity.playSound` names none, so a
//! client plays none of its own mob's sounds. The horse the rider's client
//! drives lands on both sides (`Entity.checkFallDamage` on the client,
//! `handleMoveVehicle`'s `doCheckFallDamage` on the server), so its landing
//! sound is the server's to broadcast; only the rider's own fall sound
//! (`Player.playSound`, which excludes that player) is the client's to play
//! (#16360: a rider heard the landing twice).
use super::*;
use crate::mobs::{MobTypes, equine, world::with_test_world};

/// A saddled horse with a rider on it: the client's copy of a mount it drives,
/// or the server's.
fn ridden_horse(types: &MobTypes, client_side: bool) -> Mob {
    let kind = std::sync::Arc::new(types.types["minecraft:horse"].clone());
    let mut m = Mob::new(1, kind, Vec3::ZERO, 0.0);
    m.extra.set_saddled(true);
    m.extra.client_side = client_side;
    m.rider = Some(equine::Rider::default());
    m
}

/// The events a twenty-block fall hands out: `AbstractHorse.causeFallDamage`
/// thuds (`entity.horse.land`, 0.4, 1.0), hurts the horse and hands the fall
/// to its rider.
fn landing(w: &MobWorld<'_>, types: &MobTypes, client_side: bool) -> Vec<MobEvent> {
    let mut m = ridden_horse(types, client_side);
    let mut level = crate::health::TestLevel::new(0);
    let mut events = Vec::new();
    m.cause_fall_damage_from(w.loop_context, &mut level.level(), 20.0, 1.0, &mut events);
    events
}

#[test]
fn a_clients_own_horse_leaves_its_landing_sound_to_the_server() {
    let ran = with_test_world(&[], |w, types| {
        let client = landing(w, types, true);
        assert!(
            !client
                .iter()
                .any(|e| matches!(e, MobEvent::Sound("entity.horse.land", ..))),
            "the client's copy plays no horse landing sound; the server's \
             broadcast is the one the rider hears: {client:?}"
        );
        // The rider's own fall is still the client's: `Player.playSound`
        // excludes that player, so the server does not send it to them.
        assert!(
            client
                .iter()
                .any(|e| matches!(e, MobEvent::FallToPassengers { .. })),
            "the rider's fall still reaches their client: {client:?}"
        );
        let server = landing(w, types, false);
        assert!(
            server.iter().any(|e| matches!(
                e,
                MobEvent::Sound("entity.horse.land", _, volume, pitch)
                    if *volume == 0.4 && *pitch == 1.0
            )),
            "the server's copy plays the landing it broadcasts: {server:?}"
        );
    });
    let _ = ran;
}
