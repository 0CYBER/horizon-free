//! The wither on the game side: building it (`WitherSkullBlock.checkSpawn`),
//! its boss bar, and its skulls hitting and exploding.
use super::Game;
use horizon_client::{
    hud::{HudBatch, rgba},
    mobs::{Damage, DamageKind, Species, Who},
    projectiles::{Projectile, ProjectileKind},
};

impl Game<'_> {
    fn is_wither_skull(&self, p: [i32; 3]) -> bool {
        let name = &*self.res.facts.get(self.world.block(p[0], p[1], p[2])).name;
        matches!(
            name,
            "minecraft:wither_skeleton_skull" | "minecraft:wither_skeleton_wall_skull"
        )
    }
    fn is_soul_base(&self, p: [i32; 3]) -> bool {
        let name = &*self.res.facts.get(self.world.block(p[0], p[1], p[2])).name;
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
                        self.res.facts.is_air(self.world.block(p[0], p[1], p[2]))
                    });
                if !ok {
                    continue;
                }
                for row in 0..3 {
                    for c in 0..3 {
                        let p = at(c, row);
                        let state = self.world.block(p[0], p[1], p[2]);
                        if !self.res.facts.is_air(state) {
                            self.destroy_particles(p, state);
                            self.world.set(p[0], p[1], p[2], self.world.air)?;
                        }
                    }
                }
                let base = at(1, 2);
                let at = [
                    f64::from(base[0]) + 0.5,
                    f64::from(base[1]) + 0.55,
                    f64::from(base[2]) + 0.5,
                ];
                if let Some(id) = self.spawn_mob("minecraft:wither", at, true)
                    && let Some(m) = self.mobs.iter_mut().find(|m| m.id == id)
                {
                    let yaw = if ax == 1 { 0.0 } else { 90.0 };
                    m.yaw = yaw;
                    m.body_yaw = yaw;
                    m.previous_body_yaw = yaw;
                    m.head_yaw = yaw;
                    horizon_client::mobs::wither::make_invulnerable(m);
                    let view =
                        self.mob_entity(&self.mobs.iter().find(|m| m.id == id).cloned().unwrap());
                    self.trigger_entity("minecraft:summoned_entity", "entity", &view);
                }
                return Ok(());
            }
        }
        Ok(())
    }
    /// The wither boss bars (purple, below the dragon's), within range.
    pub(super) fn draw_wither_bars(&self, batch: &mut HudBatch, mut y: f32) {
        let Some(p) = self.player.as_ref() else {
            return;
        };
        let atlas = &self.res.atlas;
        for m in self
            .mobs
            .iter()
            .filter(|m| m.species() == Species::Wither && m.alive())
        {
            if m.position.distance_squared(p.position) > 64.0 * 64.0 {
                continue;
            }
            let x = (batch.width / 2.0 - 91.0).floor();
            let progress = if m.extra.counter > 0 {
                1.0 - m.extra.counter as f32 / horizon_client::mobs::wither::INVULNERABLE as f32
            } else {
                (m.health / m.attrs.max_health).clamp(0.0, 1.0)
            };
            batch.sprite(atlas, "gui/sprites/boss_bar/purple_background", x, y);
            batch.sprite_part(
                atlas,
                "gui/sprites/boss_bar/purple_progress",
                x,
                y,
                progress,
            );
            let name = m
                .custom_name
                .clone()
                .unwrap_or_else(|| self.translate("entity.minecraft.wither", "Wither"));
            let w = HudBatch::text_width(atlas, &name);
            batch.text(
                atlas,
                &name,
                (batch.width / 2.0 - w / 2.0).floor(),
                y - 9.0,
                rgba(255, 255, 255, 255),
                true,
            );
            y += 19.0;
        }
    }
    pub(super) fn wither_near(&self) -> bool {
        self.player.as_ref().is_some_and(|p| {
            self.mobs.iter().any(|m| {
                m.species() == Species::Wither
                    && m.alive()
                    && m.position.distance_squared(p.position) <= 64.0 * 64.0
            })
        })
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
