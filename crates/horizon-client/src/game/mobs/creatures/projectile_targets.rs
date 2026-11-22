//! What a projectile can strike: the mobs, the player, the dragon's parts and
//! End crystals, and the entities that are pickable (boats, minecarts, primed
//! TNT, item frames, armor stands), with the hits on frames and stands.
use super::super::Game;
use super::super::commands::Target;
use horizon_client::arrows::EntityHit;
use horizon_client::mobs::Who;
use horizon_client::projectiles::{Projectile, ProjectileKind};
use horizon_core::geometry::{Aabb, Vec3};

impl Game<'_> {
    /// The boats, minecarts, primed TNT, item frames and armor stands a
    /// projectile can strike (`VehicleEntity.isPickable`, `PrimedTnt.isPickable`,
    /// `BlockAttachedEntity.isPickable`, `ArmorStand.isPickable`: not a marker).
    /// A stand without gravity has `noPhysics` (`ArmorStand.hasPhysics`), which
    /// only hurting projectiles skip (`drop_no_physics_stands`).
    pub(in crate::game) fn pickable_object_targets(&self) -> Vec<(Who, Aabb)> {
        let mut targets: Vec<(Who, Aabb)> = self
            .boats
            .iter()
            .map(|b| (Who::Mob(b.id), b.bounds()))
            .collect();
        targets.extend(self.carts.iter().map(|c| (Who::Mob(c.id), c.bounds())));
        targets.extend(self.tnts.iter().map(|t| (Who::Mob(t.id), t.bounds())));
        targets.extend(self.item_frames.iter().map(|f| {
            let (lo, hi) = f.bounds();
            (
                Who::Mob(f.id),
                Aabb::new(
                    Vec3::new(lo[0], lo[1], lo[2]),
                    Vec3::new(hi[0], hi[1], hi[2]),
                ),
            )
        }));
        targets.extend(
            self.armor_stands
                .iter()
                .filter(|a| !a.marker)
                .map(|a| (Who::Mob(a.id), a.bounds())),
        );
        targets
    }
    /// `AbstractHurtingProjectile.canHitEntity`: `!entity.noPhysics`. A fireball, a
    /// wither skull, a dragon fireball and a wind charge fly through an armor stand
    /// without gravity (`ArmorStand.hasPhysics`); an arrow, a trident or a thrown
    /// item still hits it. Takes those stands out of `targets` for such a kind.
    pub(in crate::game) fn drop_no_physics_stands(
        &self,
        kind: &ProjectileKind,
        targets: &mut Vec<(Who, Aabb)>,
    ) {
        if !matches!(
            kind,
            ProjectileKind::SmallFireball
                | ProjectileKind::LargeFireball { .. }
                | ProjectileKind::DragonFireball
                | ProjectileKind::WitherSkull { .. }
                | ProjectileKind::WindCharge
                | ProjectileKind::BreezeWindCharge
        ) {
            return;
        }
        targets.retain(|(who, _)| {
            !matches!(who, Who::Mob(id)
                if self.armor_stands.iter().any(|a| a.id == *id && a.no_gravity))
        });
    }
    /// A projectile's hit on armor stand `id`: `ArmorStand.hurtServer` for the
    /// projectile's damage type (`AbstractArrow`, `ThrownTrident`, `SmallFireball`,
    /// `LargeFireball`, `WitherSkull` and `WindCharge` `onHitEntity`). An arrow or
    /// a trident is then done: spent, or turned back when the stand refused it
    /// (true). Any other projectile goes on to its own hit (false): a burst, a
    /// break, an explosion. Not a stand: false, and nothing happens.
    pub(in crate::game) fn projectile_hits_stand(&mut self, p: &Projectile, id: u32) -> bool {
        if !self.armor_stands.iter().any(|a| a.id == id) {
            return false;
        }
        let damage_type = match p.kind {
            ProjectileKind::Arrow { .. } => "minecraft:arrow",
            ProjectileKind::Trident => "minecraft:trident",
            // `DamageSources.fireball`: `unattributed_fireball` without an owner.
            ProjectileKind::SmallFireball | ProjectileKind::LargeFireball { .. } => {
                if p.owner.is_some() {
                    "minecraft:fireball"
                } else {
                    "minecraft:unattributed_fireball"
                }
            }
            ProjectileKind::WitherSkull { .. } => "minecraft:wither_skull",
            ProjectileKind::WindCharge | ProjectileKind::BreezeWindCharge => {
                "minecraft:wind_charge"
            }
            // `ThrowableProjectile`s hurt with `thrown`, which a stand refuses.
            _ => "minecraft:thrown",
        };
        let causing = match p.owner {
            Some(Who::Player) => Some(Target::Player),
            Some(Who::Mob(owner)) => Some(Target::Mob(owner)),
            None => None,
        };
        if p.arrow.is_none() {
            self.command_damage_stand(id, damage_type, causing);
            return false;
        }
        // AbstractArrow.onHitEntity rolls the arrow's damage (its crit) first.
        let mut arrow = p.clone();
        self.shot_damage(&mut arrow, Some("minecraft:armor_stand"), Who::Mob(id));
        let outcome = if self.command_damage_stand(id, damage_type, causing) {
            EntityHit::Hurt
        } else {
            EntityHit::Refused
        };
        self.finish_arrow_hit(arrow, outcome);
        true
    }
    /// AbstractArrow.onHitEntity on item frame `id`: spent when the frame took
    /// the hit, turned back when a fixed one refused it (true). Not an arrow's
    /// hit on a frame: false.
    pub(in crate::game) fn arrow_hits_frame(&mut self, p: &Projectile, id: u32) -> bool {
        if p.arrow.is_none() || !self.item_frames.iter().any(|f| f.id == id) {
            return false;
        }
        let by_player = p.owner == Some(Who::Player);
        let hurt = self.projectile_hurts_frame(id, by_player, by_player && self.creative);
        let outcome = if hurt {
            EntityHit::Hurt
        } else {
            EntityHit::Refused
        };
        self.finish_arrow_hit(p.clone(), outcome);
        true
    }
    /// What a projectile can hit: the mobs, the player, the dragon's parts and
    /// the End crystals (under reserved ids, game/dragon.rs), and the vehicles
    /// and item frames.
    pub(in crate::game) fn projectile_targets(&self) -> Vec<(Who, Aabb)> {
        let mut targets: Vec<(Who, Aabb)> = self
            .mobs
            .iter()
            .filter(|m| m.alive())
            .map(|m| (Who::Mob(m.id), m.bounds()))
            .collect();
        if let Some(p) = self.player_view()
            && p.alive
        {
            targets.push((Who::Player, p.bounds));
        }
        targets.extend(
            self.special_targets()
                .into_iter()
                .map(|(id, b)| (Who::Mob(id), b)),
        );
        // Boats, minecarts, primed TNT, item frames and armor stands are
        // pickable for every projectile, so every kind shares this one list.
        targets.extend(self.pickable_object_targets());
        targets
    }
}
