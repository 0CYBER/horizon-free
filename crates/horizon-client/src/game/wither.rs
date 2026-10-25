//! The wither on the game side: building it (`WitherSkullBlock.checkSpawn`),
//! its boss bar, and its skulls hitting and exploding.
use super::Game;
use horizon_client::{
    mobs::{Damage, DamageKind, Species, Who},
    projectiles::{Projectile, ProjectileKind},
};

impl Game<'_> {
    fn is_wither_skull(&self, p: [i32; 3]) -> bool {
        let name = &*self
            .res
            .facts()
            .get(self.world.block(p[0], p[1], p[2]))
            .name;
        matches!(
            name,
            "minecraft:wither_skeleton_skull" | "minecraft:wither_skeleton_wall_skull"
        )
    }
    fn is_soul_base(&self, p: [i32; 3]) -> bool {
        let name = &*self
            .res
            .facts()
            .get(self.world.block(p[0], p[1], p[2]))
            .name;
        matches!(name, "minecraft:soul_sand" | "minecraft:soul_soil")
    }
    /// After a wither skeleton skull is placed: the T of soul sand (or soul
    /// soil) under three skulls, upright along X or Z, becomes a wither.
    pub(super) fn check_wither_spawn(&mut self, placed: [i32; 3]) -> horizon_client::Result<()> {
        if !self.is_wither_skull(placed) {
            return Ok(());
        }
        // Pattern rows top to bottom: "^^^", "###", "~#~" ('~' air).
        for (ax, az) in [(1, 0), (0, 1)] {
            for col in 0..3 {
                let left = [placed[0] - ax * col, placed[1], placed[2] - az * col];
                let at = |c: i32, row: i32| [left[0] + ax * c, left[1] - row, left[2] + az * c];
                let ok = (0..3).all(|c| self.is_wither_skull(at(c, 0)))
                    && (0..3).all(|c| self.is_soul_base(at(c, 1)))
                    && self.is_soul_base(at(1, 2))
                    && [0, 2].iter().all(|&c| {
                        let p = at(c, 2);
                        self.res.facts().is_air(self.world.block(p[0], p[1], p[2]))
                    });
                if !ok {
                    continue;
                }
                // WitherSkullBlock.checkSpawn: clear the pattern, the wither
                // appears on the middle of the bottom row, then neighbours update.
                let cells: Vec<[i32; 3]> = (0..3)
                    .flat_map(|c| (0..3).map(move |row| (c, row)))
                    .map(|(c, row)| at(c, row))
                    .collect();
                self.clear_pattern_blocks(&cells)?;
                let base = at(1, 2);
                let at = [
                    f64::from(base[0]) + 0.5,
                    f64::from(base[1]) + 0.55,
                    f64::from(base[2]) + 0.5,
                ];
                let yaw = if ax == 1 { 0.0 } else { 90.0 };
                if let Some(id) = self.create_triggered("minecraft:wither", at, yaw)
                    && let Some(m) = self.mobs.iter_mut().find(|m| m.id == id)
                {
                    m.persistent = true;
                    horizon_client::mobs::wither::make_invulnerable(m);
                    let view =
                        self.mob_entity(&self.mobs.iter().find(|m| m.id == id).cloned().unwrap());
                    self.trigger_entity("minecraft:summoned_entity", "entity", &view);
                }
                self.update_pattern_blocks(&cells)?;
                return Ok(());
            }
        }
        Ok(())
    }
    /// `WitherSkull.onHitEntity` / `onHit`: 8 damage (healing the wither 5
    /// when it kills), withering for 10 seconds, then a power 1 blast.
    pub(super) fn wither_skull_hit(
        &mut self,
        p: &Projectile,
        who: Option<Who>,
        at: [f64; 3],
    ) -> horizon_client::Result<()> {
        let owner = match p.owner {
            Some(Who::Mob(id)) => Some(id),
            _ => None,
        };
        let amount = if owner.is_some() { 8.0 } else { 5.0 };
        match who {
            Some(Who::Player) => {
                if !self.creative {
                    self.mob_hits_player_kind(
                        amount,
                        [p.previous.x, p.previous.y, p.previous.z],
                        "minecraft:wither_skull",
                    );
                    self.give_player_effect("minecraft:wither", 200, 1);
                }
            }
            Some(Who::Mob(id)) => {
                let source = Damage {
                    kind: DamageKind::Magic,
                    attacker: p.owner,
                    from: Some([p.previous.x, p.previous.y, p.previous.z]),
                };
                let killed = match self.mobs.iter_mut().find(|m| m.id == id) {
                    Some(m) => {
                        let hit = m.damage(amount, source);
                        if hit && m.alive() && m.species() != Species::Wither {
                            m.add_effect("minecraft:wither", 200, 1);
                        }
                        hit && !m.alive()
                    }
                    None => false,
                };
                if killed
                    && let Some(o) = owner
                    && let Some(w) = self.mobs.iter_mut().find(|m| m.id == o)
                {
                    w.health = (w.health + 5.0).min(w.attrs.max_health);
                }
            }
            None => {}
        }
        if let Some(id) = owner {
            self.note_attacker("minecraft:explosion", id);
        }
        self.explode(at, 1.0)
    }
}
/// Wither skulls drawn as the skull model, blue when dangerous.
pub(super) fn skull_kind(p: &Projectile) -> Option<bool> {
    match p.kind {
        ProjectileKind::WitherSkull { dangerous } => Some(dangerous),
        _ => None,
    }
}
