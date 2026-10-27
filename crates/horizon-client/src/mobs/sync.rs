//! A server's mobs as a client animates them. The server thread, a LAN host
//! and a dedicated server run each mob's animation state (`ai::tick`); a
//! client of a server only gets synched data and entity events, and runs the
//! animation steps vanilla runs on the client (`Entity.onSyncedDataUpdated`,
//! `handleEntityEvent` and the per tick animation states). The server's half
//! is `game/lan.rs` (`mob_meta`, with `pose_id` here for index 6) and
//! `MobEvent::EntityEvent`; the client's half is `game/remote_mob_data.rs`,
//! `game/remote.rs` (`C::EntityEvent`) and `client_tick` here.
//!
//! To animate another species for a client:
//! - send its synched data in `mob_meta` and read it in
//!   `remote_mob_data::apply`;
//! - send its poses in `pose_id` and read them in `set_pose`;
//! - broadcast its entity events with `MobEvent::EntityEvent` and handle
//!   them in `entity_event`;
//! - run its client side animation in `client_tick`, sharing the function
//!   with the server's `ai_step` where the server computes the same.
use super::{Mob, Species, breeze, camel, shulker, sniffer, warden};

// `Pose.id` of the poses mobs take.
pub const STANDING: u8 = 0;
pub const LONG_JUMPING: u8 = 6;
pub const SITTING: u8 = 10;
pub const ROARING: u8 = 11;
pub const SNIFFING: u8 = 12;
pub const EMERGING: u8 = 13;
pub const DIGGING: u8 = 14;
pub const SLIDING: u8 = 15;
pub const SHOOTING: u8 = 16;
pub const INHALING: u8 = 17;

/// `Entity.DATA_POSE` (index 6) of a mob of a server: the pose its species
/// keeps in its own fields.
pub fn pose_id(m: &Mob) -> u8 {
    match m.species() {
        Species::Warden => warden::pose_id(m),
        Species::Breeze => breeze::pose_id(m),
        Species::Camel | Species::CamelHusk if camel::sitting(m) => SITTING,
        _ => STANDING,
    }
}

/// `Entity.onSyncedDataUpdated(DATA_POSE)` on a client.
pub fn set_pose(m: &mut Mob, id: u8) {
    m.extra.pose = id;
    match m.species() {
        Species::Warden => warden::set_pose_id(m, id),
        Species::Breeze => breeze::set_pose_id(m, id),
        _ => {}
    }
}

/// The per tick animation steps of a client's mobs (after `m.age` moved on),
/// for the species whose server runs them in `ai_step`. `game_time` is the
/// client level's.
pub fn client_tick(m: &mut Mob, game_time: i64) {
    match m.species() {
        Species::Warden => warden::client_tick(m),
        Species::Sniffer => sniffer::animate(m),
        Species::Breeze => breeze::ai_step(m),
        Species::Camel | Species::CamelHusk => camel::client_tick(m, game_time),
        Species::Shulker => shulker::client_tick(m),
        _ => {}
    }
}

/// `handleEntityEvent` of the mob types that start animations by an event
/// (`ClientboundEntityEventPacket` for a mob of a client).
pub fn entity_event(m: &mut Mob, event: u8) {
    let age = m.age as f32;
    if m.species() == Species::Warden {
        match event {
            // Attack: the roar ends, the swing starts.
            4 => {
                m.anim.states[warden::ROAR].stop();
                m.anim.states[warden::ATTACK].start(age);
            }
            // The tendrils twitch.
            61 => m.anim.v[warden::TENDRIL] = 10.0,
            // The sonic boom.
            62 => m.anim.states[warden::SONIC_BOOM].start(age),
            _ => {}
        }
    }
}
