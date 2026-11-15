//! A player's reach and flight (#3349): whether an entity is within the server's reach rules,
//! and whether the player may fly (NeoForge's `IPlayerExtension.mayFly`).
use super::Game;
use horizon_core::geometry::{Aabb, Vec3};
use mod_api::{EntityReach, EntityRef, Hand, LivingRef, PlayerRef};

impl Game<'_> {
    /// The server's reach rule `reach` between `player` and `entity`: the squared distance from
    /// the player's eyes to the closest point of the entity's box (`AABB.distanceToSqr`, 0 with
    /// the eyes inside), against the player's `entity_interaction_range` with the rule's buffer,
    /// or for an attack the main hand's attack range (`Player.isWithinAttackRange`). None for a
    /// stale player or entity, one without a box, or one in another level.
    pub(super) fn player_within_entity_reach(
        &self,
        player: PlayerRef,
        entity: EntityRef,
        reach: EntityReach,
    ) -> Option<bool> {
        let me = player.entity();
        if self.entity_level(me)? != self.entity_level(entity)? {
            return None;
        }
        let v = |[x, y, z]: [f64; 3]| Vec3::new(x, y, z);
        let [lo, hi] = self.entity_bounding_box(entity)?;
        let eye = v(self.entity_eye_position(me)?);
        let distance_squared = Aabb::new(v(lo), v(hi)).distance_squared_to_point(eye);
        let living = LivingRef::new(me);
        let range = self.living_attribute_value(living, "minecraft:entity_interaction_range")?;
        // The server's buffers: 3.0 on its interact and attack checks, 4.0 for a container an
        // entity holds (`ContainerEntity.isChestVehicleStillValid`, the mount inventory).
        let within = |buffer: f64| distance_squared < (range + buffer) * (range + buffer);
        Some(match reach {
            EntityReach::Interact => within(3.0),
            EntityReach::Container => within(4.0),
            EntityReach::Attack => {
                let creative = self.player_is_creative(player)?;
                let held = self.living_hand_item(living, Hand::Main);
                self.stack_attack_range(held.as_ref(), true, creative, range)
                    .contains(distance_squared.sqrt(), 3.0)
            }
        })
    }

    /// `IPlayerExtension.mayFly`: whether the player may start flying, by game mode or ability.
    /// Horizon has no creative-flight attribute yet, so this is the `mayfly` ability. None for a
    /// stale or unavailable player.
    pub(super) fn player_may_fly(&self, player: PlayerRef) -> Option<bool> {
        Some(self.player_abilities(player)?.may_fly)
    }
}

#[cfg(test)]
mod tests {
    use crate::game::native_guest_turn_tests::with_guests;
    use horizon_client::mobs::Mob;
    use horizon_core::geometry::Vec3;
    use mod_api::{EntityReach, EntityRef};
    use std::sync::Arc;

    /// Each reach rule on both sides of its boundary, for a survival guest with an empty hand
    /// (range 3.0, no attack range component) whose eyes are at (3, 65.62, 5). Pigs stand level
    /// with the eyes on +x; a pig's half-width is `0.9f32 / 2` = 0.449999988079071044921875, so
    /// a pig whose feet are at `3 + d + half` has its near face exactly `d` from the eyes.
    /// Interact is strict below 6.0, the entity container strict below 7.0, the attack inclusive
    /// up to 6.0. With the eyes inside the box the distance is 0 and every rule passes. A handle
    /// that names nothing, and a guest of the hub in another level, answer None. A player in
    /// survival may not fly until the `mayfly` ability is set.
    #[test]
    #[ignore = "daily"]
    fn reach_runs_from_the_eyes_to_the_closest_point_and_flight_follows_the_ability() {
        with_guests(|game| {
            let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            let eye_y = 64.0 + 1.62;
            let half = f64::from(0.9f32) / 2.0;
            let pig_at = |game: &mut crate::game::Game<'_>, id: u32, face: f64| {
                let feet = Vec3::new(3.0 + face + half, eye_y - 0.45, 5.0);
                game.mobs.push(Mob::new(id, kind.clone(), feet, 0.0));
                let pig = game.entity(id).unwrap();
                let [lo, _] = game.entity_bounding_box(pig).unwrap();
                assert_eq!(lo[0], 3.0 + face, "pig {id}'s near face");
                pig
            };
            let player = game.player(12).unwrap();
            let reach = |game: &crate::game::Game<'_>, pig, rule| {
                game.player_within_entity_reach(player, pig, rule)
            };
            let inside = pig_at(game, 900, -half);
            let short = pig_at(game, 901, 5.5);
            let six = pig_at(game, 902, 6.0);
            let beyond = pig_at(game, 903, 6.5);
            let seven = pig_at(game, 904, 7.0);

            assert_eq!(reach(game, short, EntityReach::Interact), Some(true));
            assert_eq!(reach(game, six, EntityReach::Interact), Some(false));
            assert_eq!(reach(game, beyond, EntityReach::Container), Some(true));
            assert_eq!(reach(game, seven, EntityReach::Container), Some(false));
            assert_eq!(reach(game, six, EntityReach::Attack), Some(true));
            assert_eq!(reach(game, beyond, EntityReach::Attack), Some(false));
            for rule in [
                EntityReach::Interact,
                EntityReach::Container,
                EntityReach::Attack,
            ] {
                assert_eq!(
                    reach(game, inside, rule),
                    Some(true),
                    "{rule:?}, eyes inside"
                );
            }
            // A handle that names nothing real: None.
            let stale = EntityRef::new(short.epoch(), 424242);
            assert_eq!(reach(game, stale, EntityReach::Interact), None);

            // A guest of the hub whose level is the nether: another level, None.
            use horizon_client::lan::{Joined, PlayerSender, guest::Guest};
            use horizon_protocol::packets::configuration::ClientInformation;
            let (tx, _rx) = std::sync::mpsc::channel();
            use std::sync::atomic::{AtomicI32, AtomicUsize};
            let mut guest = Guest::new(
                Joined {
                    conn: 9,
                    uuid: 7777,
                    name: "NetherGuest".to_owned(),
                    properties: Vec::new(),
                    info: ClientInformation::new(8),
                    brand: String::new(),
                    sender: PlayerSender::new(tx, Arc::new(AtomicUsize::new(0))),
                    latency: Arc::new(AtomicI32::new(0)),
                    address: "127.0.0.1:1".parse().unwrap(),
                    expected: false,
                },
                777,
                [0.0, 64.0, 0.0],
                10,
            );
            guest.placed = true;
            guest.dimension = "minecraft:the_nether".to_owned();
            game.lan.as_mut().unwrap().hub.guests.push(guest);
            game.reset_entity_residency();
            let nether = game.entity(777).unwrap();
            assert_eq!(reach(game, nether, EntityReach::Interact), None);

            assert_eq!(game.player_may_fly(player), Some(false));
            let mut abilities = game.player_abilities(player).unwrap();
            abilities.may_fly = true;
            assert!(game.player_set_abilities(player, abilities));
            assert_eq!(game.player_may_fly(player), Some(true));
        });
    }
}
