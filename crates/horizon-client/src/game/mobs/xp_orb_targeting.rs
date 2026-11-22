//! `xp_orb_targeting` (NeoForge's `XpOrbTargetingEvent`): the player an
//! experience orb goes for.
use super::Game;

/// A player an orb may follow: the feet, the point the orb homes on and the
/// player's UUID (None for a host with no saved UUID).
pub(super) type OrbCandidate = ([f64; 3], [f64; 3], Option<u128>, u32);

/// `ExperienceOrb.followNearbyPlayer`'s scan distance.
const SCAN_DISTANCE: f64 = 8.0;

impl Game<'_> {
    /// The point orb `i` homes on this tick (`ExperienceOrb.followNearbyPlayer`):
    /// the nearest of `followed` within 8 blocks, unless the mods choose
    /// another player or none.
    pub(super) fn orb_target(&mut self, i: usize, followed: &[OrbCandidate]) -> Option<[f64; 3]> {
        let o = &self.orbs[i];
        let (id, at, join) = (o.id, o.position, o.join);
        let distance = |feet: &[f64; 3]| -> f64 {
            (feet[0] - at.x).powi(2) + (feet[1] - at.y).powi(2) + (feet[2] - at.z).powi(2)
        };
        let kept = o
            .following
            .and_then(|id| followed.iter().find(|c| c.3 == id))
            .filter(|c| distance(&c.0) <= SCAN_DISTANCE * SCAN_DISTANCE);
        let nearest = kept.or_else(|| {
            followed
                .iter()
                .filter(|c| distance(&c.0) < SCAN_DISTANCE * SCAN_DISTANCE)
                .min_by(|a, b| distance(&a.0).total_cmp(&distance(&b.0)))
        });
        self.orbs[i].following = nearest.map(|c| c.3);
        if !self.native_authoritative()
            || !mod_api::__private::listened(mod_api::HookId::XpOrbTargeting)
        {
            return nearest.map(|c| c.1);
        }
        let Some(epoch) = self.native_lifecycle.epoch() else {
            return nearest.map(|c| c.1);
        };
        let orb = mod_api::EntityRef::new(epoch, id);
        let found = nearest.and_then(|c| c.2).and_then(|uuid| self.player(uuid));
        let mut following = found;
        self.native_xp_orb_targeting(orb, SCAN_DISTANCE, &mut following);
        let i = self.orbs.locate(join)?;
        if following == found {
            return nearest.map(|c| c.1);
        }
        // Vanilla keeps the answer only for a living player who isn't a
        // spectator: the candidates are exactly those.
        let chosen = following.and_then(|p| followed.iter().find(|c| c.2 == Some(p.uuid())));
        self.orbs[i].following = chosen.map(|c| c.3);
        chosen.map(|c| c.1)
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_entity_refs::tests::with_game;
    use super::super::native_item_places::tests::{guest, open_to_lan};
    use horizon_client::entities::ExperienceOrb;
    use horizon_core::geometry::Vec3;
    use mod_api::{Hooks, ModEntry, ServerHost, XpOrbTargetingEvent};
    use std::sync::{Arc, Mutex};

    /// What the hook heard: the orb's id, the scan distance and the default
    /// player's UUID.
    type Heard = Arc<Mutex<Vec<(u32, f64, Option<u128>)>>>;

    /// Sends orb 900 to the far guest (UUID 124) and orb 901 to no one.
    struct Shepherd(Heard);
    #[mod_api::__subscriptions]
    impl Hooks for Shepherd {
        fn xp_orb_targeting(&self, host: &mut dyn ServerHost, e: &mut XpOrbTargetingEvent<'_>) {
            let row = (e.orb.id(), e.scan_distance, e.following.map(|p| p.uuid()));
            self.0.lock().unwrap().push(row);
            match e.orb.id() {
                900 => *e.following = host.player(124),
                901 => *e.following = None,
                _ => {}
            }
        }
    }

    fn orb(id: u32, at: [f64; 3]) -> ExperienceOrb {
        let at = Vec3::new(at[0], at[1], at[2]);
        ExperienceOrb {
            fire_ticks: 0,
            attachment_lifetime: Default::default(),
            fall_distance: 0.0,
            rotation: [0.0, 0.0],
            join: Default::default(),
            section_order: 0,
            no_gravity: false,
            id,
            position: at,
            previous: at,
            velocity: Vec3::new(0.0, 0.0, 0.0),
            value: 1,
            age: 0,
            on_ground: false,
            count: 1,
            ticks: 0,
            air: 300,
            health: 5,
            following: None,
        }
    }

    /// `XpOrbTargetingEvent`: each orb asks with the nearest guest as the
    /// default; a mod sends one orb to a guest out of range and stops another,
    /// and an orb it leaves alone keeps the default.
    #[test]
    #[ignore = "daily"]
    fn a_mod_chooses_whom_each_orb_follows() {
        let heard = Heard::default();
        let mods = vec![ModEntry::new("shepherd", Box::new(Shepherd(heard.clone())))].into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                open_to_lan(game);
                let guests = &mut game.lan.as_mut().unwrap().hub.guests;
                for (conn, x) in [(1, 0.0), (2, 30.0)] {
                    let mut g = guest(conn);
                    g.loaded = true;
                    g.position = [x, 64.0, 0.0];
                    guests.push(g);
                }
                for id in [900, 901, 902] {
                    game.orbs.push(orb(id, [1.0, 64.0, 0.0]));
                }
                let followed = game.orb_followed();
                let half = game.lan.as_ref().unwrap().hub.guests[0].eye_height() / 2.0;
                let eye = |x: f64| [x, 64.0 + half, 0.0];
                let targets: Vec<_> = (0..3).map(|i| game.orb_target(i, &followed)).collect();
                assert_eq!(targets, [Some(eye(30.0)), None, Some(eye(0.0))]);
                let default = (8.0, Some(123));
                let heard = heard.lock().unwrap();
                assert_eq!(*heard, [900, 901, 902].map(|id| (id, default.0, default.1)));
            });
        });
    }
}
