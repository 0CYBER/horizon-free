//! Mobs and the guests of an open world. The mob code sees one player
//! (`MobWorld.player`, `Who::Player`); with guests on, each mob sees the
//! player vanilla's `Level.getNearestPlayer` would give it, and keeps the
//! guest it targets until it lets go. What it then does to `Who::Player`
//! (hits, pushes, fire, its projectiles) runs with that guest swapped in.
use super::{Game, lan::LanHost};
use horizon_client::{
    mobs::{Mob, MobEvent, PlayerView, Who},
    projectiles::{Hit, Projectile},
};
use horizon_core::geometry::{Aabb, Vec3};
use horizon_protocol::packets::play::{self, Clientbound as C};

/// The guests mobs can see this tick, by connection.
pub(super) type GuestViews = Vec<(u32, PlayerView)>;

/// The player a mob sees this tick: `None` for the host, else a guest.
/// A mob that targets a guest, or that a guest hurt in the last 100 ticks
/// (`LivingEntity.getLastHurtByMob`), keeps them while they are still a
/// valid target; otherwise it sees the nearest player
/// (`TargetingConditions`: alive, neither creative nor spectator), or the
/// nearest one that is not a spectator if none qualifies.
pub(super) fn player_for(
    m: &Mob,
    host: Option<&PlayerView>,
    guests: &GuestViews,
    bound: Option<u32>,
) -> Option<u32> {
    let targeting = m.ai.target == Some(Who::Player)
        || m.ai.last_hurt_by == Some(Who::Player) && m.age as i32 - m.ai.last_hurt_by_time <= 100;
    choose(m.position, targeting, host, guests, bound)
}

fn choose(
    at: Vec3,
    targeting: bool,
    host: Option<&PlayerView>,
    guests: &GuestViews,
    bound: Option<u32>,
) -> Option<u32> {
    let targetable = |v: &PlayerView| v.alive && !v.creative && !v.spectator;
    if targeting
        && let Some(conn) = bound
        && guests.iter().any(|(c, v)| *c == conn && targetable(v))
    {
        return Some(conn);
    }
    let d = |v: &PlayerView| at.distance_squared(v.position);
    let nearest = |ok: &dyn Fn(&PlayerView) -> bool| {
        let mut best = host.filter(|v| ok(v)).map(|v| (None, d(v)));
        for (conn, v) in guests.iter().filter(|(_, v)| ok(v)) {
            if best.is_none_or(|(_, b)| d(v) < b) {
                best = Some((Some(*conn), d(v)));
            }
        }
        best
    };
    nearest(&targetable)
        .or_else(|| nearest(&|v: &PlayerView| v.alive && !v.spectator))
        .and_then(|(conn, _)| conn)
}

/// The events of a mob's tick that act on the player it sees.
pub(super) fn acts_on_player(e: &MobEvent) -> bool {
    matches!(
        e,
        MobEvent::AttackPlayer(..)
            | MobEvent::DamagePlayer { .. }
            | MobEvent::Push {
                target: Who::Player,
                ..
            }
            | MobEvent::Ignite {
                target: Who::Player,
                ..
            }
            | MobEvent::Launch {
                target: Who::Player,
                ..
            }
    )
}

impl Game<'_> {
    /// How mobs see each guest in play (`player_view` with the guest swapped
    /// in; what the host alone tracks, such as its last attacker, stays out).
    pub(super) fn lan_guest_views(&mut self) -> GuestViews {
        let Some(mut host) = self.lan.take() else {
            return Vec::new();
        };
        let conns: Vec<(u32, u128, [f64; 3])> = host
            .hub
            .guests
            .iter()
            .filter(|g| g.placed && g.loaded && !g.dead)
            .map(|g| {
                let moved = [0, 1, 2].map(|k| g.position[k] - g.previous_position[k]);
                (g.conn, g.uuid, moved)
            })
            .collect();
        let mut views = Vec::new();
        for (conn, uuid, moved) in conns {
            if let Some(Some(mut v)) = self.as_guest(&mut host, conn, |g| g.player_view()) {
                v.hurt_by = None;
                v.last_hurt = None;
                v.hero = false;
                v.velocity = Vec3::new(moved[0], moved[1], moved[2]);
                v.uuid = horizon_client::lan::playerdata::uuid_ints(uuid);
                v.allies = Default::default();
                views.push((conn, v));
            }
        }
        self.lan = Some(host);
        views
    }

    /// Mob events aimed at a guest, run as that guest. Knockback reaches
    /// their client (`hurtMarked`: ClientboundSetEntityMotionPacket).
    pub(super) fn lan_guest_mob_events(&mut self, events: Vec<(u32, MobEvent)>) {
        if events.is_empty() {
            return;
        }
        let Some(mut host) = self.lan.take() else {
            return;
        };
        for (conn, e) in events {
            let pushed = self.as_guest(&mut host, conn, |g| {
                let before = g.player.as_ref().map(|p| p.velocity);
                let _ = g.handle_mob_events(vec![e]);
                g.player
                    .as_ref()
                    .map(|p| p.velocity)
                    .filter(|v| Some(*v) != before)
            });
            if let Some(Some(v)) = pushed {
                send_motion(&host, conn, v);
            }
        }
        self.lan = Some(host);
    }

    /// The guests a projectile can hit, under their entity ids.
    pub(super) fn lan_projectile_targets(&self) -> Vec<(Who, Aabb)> {
        let Some(host) = self.lan.as_ref() else {
            return Vec::new();
        };
        host.hub
            .guests
            .iter()
            .filter(|g| g.placed && g.loaded && !g.dead && !g.spectator())
            .map(|g| {
                let [x, y, z] = g.position;
                let height = if g.sneaking { 1.5 } else { 1.8 };
                let b = Aabb::new(
                    Vec3::new(x - 0.3, y, z - 0.3),
                    Vec3::new(x + 0.3, y + height, z + 0.3),
                );
                (Who::Mob(g.entity_id as u32), b)
            })
            .collect()
    }

    /// A projectile that hit a guest: its hit runs as theirs. False when
    /// `id` is not a guest.
    pub(super) fn lan_projectile_hit(&mut self, p: &Projectile, id: u32, at: Vec3) -> bool {
        let Some(mut host) = self.lan.take() else {
            return false;
        };
        let conn = host
            .hub
            .guests
            .iter()
            .find(|g| g.entity_id as u32 == id)
            .map(|g| g.conn);
        if let Some(conn) = conn {
            let pushed = self.as_guest(&mut host, conn, |g| {
                let before = g.player.as_ref().map(|p| p.velocity);
                let hit = Hit::Entity {
                    who: Who::Player,
                    at,
                };
                let _ = g.projectile_hit(p, hit);
                g.player
                    .as_ref()
                    .map(|p| p.velocity)
                    .filter(|v| Some(*v) != before)
            });
            if let Some(Some(v)) = pushed {
                send_motion(&host, conn, v);
            }
        }
        self.lan = Some(host);
        conn.is_some()
    }
}

fn send_motion(host: &LanHost, conn: u32, v: Vec3) {
    if let Some(g) = host.hub.guest(conn) {
        g.send(&C::SetEntityMotion(play::SetEntityMotion {
            id: g.entity_id,
            velocity: [v.x, v.y, v.z],
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(x: f64, creative: bool) -> PlayerView {
        PlayerView {
            position: Vec3::new(x, 64.0, 0.0),
            bounds: Aabb::new(Vec3::new(x, 64.0, 0.0), Vec3::new(x, 65.8, 0.0)),
            eye: [x, 65.62, 0.0],
            alive: true,
            creative,
            sneaking: false,
            look: [0.0; 3],
            main_hand: None,
            off_hand: None,
            head: None,
            armor_cover: 0.0,
            spectator: false,
            gold_armor: false,
            hurt_by: None,
            last_hurt: None,
            velocity: Vec3::ZERO,
            yaw: 0.0,
            hero: false,
            uuid: [0; 4],
            allies: Default::default(),
        }
    }

    const MOB: Vec3 = Vec3::new(0.0, 64.0, 0.0);

    #[test]
    fn the_nearest_survival_player_is_seen() {
        let host = view(10.0, false);
        let guests = vec![(7, view(3.0, false)), (8, view(-5.0, false))];
        assert_eq!(choose(MOB, false, Some(&host), &guests, None), Some(7));
        // A creative guest is passed over for the host further away.
        let guests = vec![(7, view(3.0, true))];
        assert_eq!(choose(MOB, false, Some(&host), &guests, None), None);
        // Nobody targetable: the nearest one is still seen.
        let host = view(10.0, true);
        assert_eq!(choose(MOB, false, Some(&host), &guests, None), Some(7));
        assert_eq!(choose(MOB, false, None, &Vec::new(), None), None);
    }

    #[test]
    fn a_targeted_guest_stays_the_target() {
        let host = view(1.0, false);
        let guests = vec![(7, view(12.0, false))];
        assert_eq!(choose(MOB, true, Some(&host), &guests, Some(7)), Some(7));
        // Gone creative: the mob looks again.
        let guests = vec![(7, view(12.0, true))];
        assert_eq!(choose(MOB, true, Some(&host), &guests, Some(7)), None);
        // Without a target the nearest player wins.
        let guests = vec![(7, view(12.0, false))];
        assert_eq!(choose(MOB, false, Some(&host), &guests, Some(7)), None);
    }
}
