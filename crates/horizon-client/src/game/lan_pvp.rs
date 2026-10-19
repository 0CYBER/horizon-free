//! Players hitting players in an open world (`Player.attack` on a player):
//! the host hits a guest, a guest hits the host or another guest. The hit
//! is worked out as the attacker (weapon, attack strength, critical hit,
//! enchantments, a mace's smash) and lands on the victim like any player
//! attack (blocking, armour, knockback), when the `pvp` game rule allows it
//! (`ServerPlayer.canHarmPlayer`, `isPvpAllowed`).
use super::{Game, lan::LanHost, lan_mobs::guest_bounds, ray_box, view_vector};

/// Who a player's attack lands on.
#[derive(Clone, Copy)]
pub(super) enum Victim {
    Host,
    Guest(u32),
}

/// The attacker's side of the hit.
pub(super) struct Strike {
    amount: f32,
    scale: f32,
    crit: bool,
    from: [f64; 3],
    sweep: Option<[f64; 3]>,
    /// The attacker's team, for the victim's `Player.canHarmPlayer`.
    team: Option<horizon_client::scoreboard::Team>,
}

impl Game<'_> {
    /// The guest on the host's crosshair and how far, within reach.
    pub(super) fn guest_hit(&self) -> Option<(u32, f64)> {
        let host = self.lan.as_ref()?;
        let eye = self.eye_position()?;
        let dir = view_vector(self.yaw, self.pitch);
        let reach = if self.creative { 5.0 } else { 3.0 };
        host.hub
            .guests
            .iter()
            .filter(|g| host.hub.here(g) && g.loaded && !g.dead && !g.spectator())
            .filter_map(|g| Some((g.conn, ray_box(eye, dir, reach, guest_bounds(g))?)))
            .min_by(|a, b| a.1.total_cmp(&b.1))
    }

    /// The host's attack on the guest under the crosshair, when no mob or
    /// block is nearer.
    pub(super) fn attack_guest(&mut self) -> bool {
        let Some((conn, d)) = self.guest_hit() else {
            return false;
        };
        if self.mob_hit().is_some_and(|(_, m)| m < d) || !self.before_block(d) {
            return false;
        }
        let Some(mut host) = self.lan.take() else {
            return false;
        };
        if let Some(strike) = self.pvp_strike() {
            self.pvp_land(&mut host, Victim::Guest(conn), &strike);
        }
        self.lan = Some(host);
        true
    }

    /// The player `id` that guest `conn` attacks, if it is one.
    pub(super) fn pvp_victim(host: &LanHost, conn: u32, id: i32) -> Option<Victim> {
        if id == host.host_entity {
            return Some(Victim::Host);
        }
        host.hub
            .guests
            .iter()
            .find(|g| g.entity_id == id && g.conn != conn && host.hub.here(g) && !g.dead)
            .map(|g| Victim::Guest(g.conn))
    }

    /// The attacker's side of `Player.attack` (the host, or a guest swapped
    /// in): the damage, the charge reset, the exhaustion and the swing.
    pub(super) fn pvp_strike(&mut self) -> Option<Strike> {
        let p = self.player.as_ref()?;
        let from = [p.position.x, p.position.y, p.position.z];
        let (amount, scale, crit) = self.melee_damage("minecraft:player");
        let sweep = self.sweep_attack_position(scale, crit);
        self.attack_ticks = 0;
        if let Some(p) = self.player.as_mut()
            && !self.creative
        {
            p.cause_food_exhaustion(0.1);
        }
        self.swing_arm();
        Some(Strike {
            amount,
            scale,
            crit,
            from,
            sweep,
            team: self.player_team().cloned(),
        })
    }

    /// The hit landing on the victim, then the attacker's attack sound.
    pub(super) fn pvp_land(&mut self, host: &mut LanHost, victim: Victim, s: &Strike) {
        // ServerPlayer.canHarmPlayer: the pvp game rule, then the victim's
        // team (Player.canHarmPlayer: allies only with friendly fire on).
        let team = match victim {
            Victim::Host => self.player_team(),
            Victim::Guest(conn) => host
                .hub
                .guest(conn)
                .and_then(|g| self.score.board.players_team(&g.name)),
        };
        let harm = horizon_client::scoreboard::teams_allow_damage(team, s.team.as_ref());
        let hurt = self.ui.rules.bool("pvp")
            && harm
            && match victim {
                Victim::Host => self.mob_hits_player(s.amount, s.from, "minecraft:player_attack"),
                Victim::Guest(conn) => self
                    .guest_hit_by(host, conn, |g| {
                        g.mob_hits_player(s.amount, s.from, "minecraft:player_attack")
                    })
                    .unwrap_or(false),
            };
        let sound = match (hurt, s.crit, s.scale > 0.9) {
            (false, ..) => "minecraft:entity.player.attack.nodamage",
            (true, true, _) => "minecraft:entity.player.attack.crit",
            (true, false, true) => "minecraft:entity.player.attack.strong",
            (true, false, false) => "minecraft:entity.player.attack.weak",
        };
        if let Some(at) = s.sweep.filter(|_| hurt) {
            self.sweep_attack(at, Some(s.from));
        } else {
            self.audio.play(sound, Some(s.from), 1.0, 1.0);
        }
    }
}
