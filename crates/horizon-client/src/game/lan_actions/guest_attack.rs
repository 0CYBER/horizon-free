//! `ServerGamePacketListenerImpl.handleAttack` for a guest: `Player.attack` on the entity the
//! packet names.
use super::*;

impl Game<'_> {
    pub(super) fn guest_attack(&mut self, host: &mut LanHost, conn: u32, id: i32) {
        let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) else {
            return;
        };
        if host.hub.guests[gi].spectator() {
            return;
        }
        if self.guest_attack_stops(host, gi, id) {
            return;
        }
        // Native mods, where Player.attack starts: a canceled blow does
        // nothing (game/native_hooks.rs).
        let aimed = if let Some(g) = host
            .hub
            .guests
            .iter()
            .find(|g| g.entity_id == id && g.conn != conn && host.hub.here(g))
        {
            Some(("minecraft:player".to_owned(), g.position))
        } else {
            // The mod hooks hear no natural fence knot (#13036).
            self.level_entity(id)
                .filter(|t| !matches!(t, crate::game::commands::Target::Knot(_)))
                .map(|t| {
                    (
                        self.target_type(t),
                        self.target_position(t).unwrap_or_default(),
                    )
                })
        };
        if let Some((kind, at)) = aimed
            && self.as_guest(host, conn, |g| {
                let mut canceled = false;
                g.native_attack_entity(id, &kind, at, &mut canceled);
                canceled
            }) == Some(true)
        {
            return;
        }
        if let Some(victim) = Self::pvp_victim(host, conn, id) {
            if let Some(Some(strike)) = self.as_guest(host, conn, |g| g.pvp_strike())
                && self.pvp_land(host, victim, &strike)
            {
                let on_ground = Self::pvp_victim_on_ground(host, victim);
                let stops_sprint = strike.stops_sprint();
                let sent = self.as_guest(host, conn, |g| g.pvp_hit_after(on_ground, stops_sprint));
                self.send_guest_motion(host, conn, sent.flatten());
            }
            return;
        }
        // A ghast's fireball or a wind charge: turned along the guest's own
        // view (ServerGamePacketListenerImpl.handleInteract, then
        // Player.attack's AIM_DEFLECT).
        if let Some(i) = self
            .creatures
            .projectiles
            .iter()
            .position(|p| p.id != 0 && p.id as i32 == id && crate::game::deflect::attackable(p))
        {
            self.as_guest_with_host(host, conn, |g, host| {
                if g.within_interaction_range(i) {
                    if matches!(
                        g.creatures.projectiles[i].kind,
                        horizon_client::projectiles::ProjectileKind::ShulkerBullet { .. }
                    ) {
                        g.hit_shulker_bullet(Some(host), i);
                    } else {
                        g.deflect(i);
                    }
                }
            });
            return;
        }
        if let Some(index) = self
            .mobs
            .iter()
            .position(|m| m.id as i32 == id && m.alive())
            .filter(|_| self.dragon.as_ref().is_none_or(|d| d.id as i32 != id))
        {
            if host.hub.guests[gi].vehicle == Some(self.mobs[index].id) {
                return;
            }
            // A mace's smash sets the attacker's velocity and sends it at once, before the
            // Wind Burst's push (MaceItem.hurtEnemy, then the post-attack effects).
            let sent = self
                .as_guest(host, conn, |g| {
                    let marked = g
                        .player
                        .as_mut()
                        .is_some_and(|p| std::mem::take(&mut p.health.sync_velocity));
                    g.attack_mob(index);
                    g.swing_arm();
                    let p = g.player.as_mut()?;
                    let sent = std::mem::replace(&mut p.health.sync_velocity, marked);
                    sent.then_some(p.velocity)
                })
                .flatten();
            self.send_guest_motion(host, conn, sent);
            // The mob now answers to this guest (game/lan_mobs.rs).
            if let Ok(id) = u32::try_from(id) {
                host.mob_players.insert(id, conn);
            }
            return;
        }
        let (old_yaw, old_pitch) = (host.hub.guests[gi].yaw, host.hub.guests[gi].pitch);
        let boat = self.boats.iter().position(|boat| boat.id as i32 == id);
        // A packet names its target already. Keep the attacker's actual yaw
        // for the boat's knockback instead of aiming a ray at its center.
        if boat.is_none() && self.look_at_entity(host, gi, id, None).is_none() {
            return;
        }
        // A mace's smash sets the attacker's velocity and sends it at once, before the
        // Wind Burst's push (MaceItem.hurtEnemy, then the post-attack effects).
        let sent = self
            .as_guest(host, conn, |g| {
                let marked = g
                    .player
                    .as_mut()
                    .is_some_and(|p| std::mem::take(&mut p.health.sync_velocity));
                if let Some(index) = boat {
                    g.strike_boat(index);
                    g.swing_arm();
                } else {
                    g.attack();
                }
                let p = g.player.as_mut()?;
                let sent = std::mem::replace(&mut p.health.sync_velocity, marked);
                sent.then_some(p.velocity)
            })
            .flatten();
        self.send_guest_motion(host, conn, sent);
        // The mob now answers to this guest (game/lan_mobs.rs).
        if let Ok(id) = u32::try_from(id) {
            host.mob_players.insert(id, conn);
        }
        host.hub.guests[gi].yaw = old_yaw;
        host.hub.guests[gi].pitch = old_pitch;
    }
}
