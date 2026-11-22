//! What a cactus does to the entities that are not living, and the
//! `entity_damage` game event of their hits: `CactusBlock.entityInside` hurts
//! every entity inside it, so a boat (`AbstractBoat.tick`'s two
//! `applyEffectsFromBlocks`), a minecart (its behaviour's) and an experience
//! orb (`ExperienceOrb.tick`'s) wear down in one, and `VehicleEntity.hurtServer`
//! and `ItemEntity.hurtServer` post `entity_damage` for every hit they take.
use super::commands::Target;
use super::*;
use horizon_client::mobs::Who;
use horizon_client::projectiles::Projectile;

/// The entity a projectile's damage names (`Projectile.getOwner`): a server's
/// guest by connection, else the owner.
pub(super) fn projectile_by(p: &Projectile) -> Option<Target> {
    match (p.guest, p.owner) {
        (Some(conn), _) => Some(Target::Guest(conn)),
        (None, Some(Who::Player)) => Some(Target::Player),
        (None, Some(Who::Mob(id))) => Some(Target::Mob(id)),
        (None, None) => None,
    }
}

/// The blocks a box less 1e-5 is in whose name is `name`: what
/// `Entity.checkInsideBlocks` visits, each block once.
fn cells_named(game: &Game, bounds: Aabb, name: &str) -> Vec<[i32; 3]> {
    let e = f64::from(1.0e-5_f32);
    let b = bounds.deflate(Vec3::new(e, e, e));
    let (lo, hi) = (b.min_position(), b.max_position());
    let facts = game.res.facts();
    let mut cells = Vec::new();
    for x in lo.x.floor() as i32..=hi.x.floor() as i32 {
        for y in lo.y.floor() as i32..=hi.y.floor() as i32 {
            for z in lo.z.floor() as i32..=hi.z.floor() as i32 {
                if &*facts.get(game.world.block(x, y, z)).name == name {
                    cells.push([x, y, z]);
                }
            }
        }
    }
    cells
}

impl Game<'_> {
    /// `VehicleEntity.hurtServer`'s `gameEvent(ENTITY_DAMAGE, source.getEntity())`
    /// at the vehicle, after the damage is added and before it breaks.
    pub(super) fn vehicle_damage_event(&mut self, at: [f64; 3], by: Option<Target>) {
        self.sculk_prepare();
        self.entity_event_by("minecraft:entity_damage", at, by);
    }
    /// `ItemEntity.hurtServer`'s `gameEvent(ENTITY_DAMAGE, source.getEntity())`
    /// at the item, before a hit that empties its health destroys it.
    pub(super) fn item_damage_event(&mut self, at: Vec3, by: Option<Target>) {
        self.sculk_prepare();
        self.entity_event_by("minecraft:entity_damage", [at.x, at.y, at.z], by);
    }
    /// [`Self::hurt_boat`] for a source that names an entity.
    pub(super) fn hurt_boat_by(
        &mut self,
        i: usize,
        damage: f32,
        creative: bool,
        by: Option<Target>,
    ) -> bool {
        self.hurt_boat_result(i, damage, creative, by)
            .unwrap_or(false)
    }
    /// The cactus hits of one of `AbstractBoat.tick`'s `applyEffectsFromBlocks`:
    /// `CactusBlock.entityInside` hurts the boat for 1 in every cactus its box
    /// meets on its move (`steps`: where it started, then where it got to),
    /// each once, while `Entity.checkInsideBlocks` walks them, before the
    /// fire the walk queues. True when a hit broke it.
    pub(super) fn boat_cactus_pass(&mut self, i: usize, steps: &[Aabb]) -> bool {
        let mut seen: Vec<[i32; 3]> = Vec::new();
        for &bounds in steps {
            for cell in cells_named(self, bounds, "minecraft:cactus") {
                if seen.contains(&cell) {
                    continue;
                }
                seen.push(cell);
                if self.burn_boat(i, 1.0, "minecraft:cactus") {
                    return true;
                }
            }
        }
        false
    }
    /// `ExperienceOrb.tick`'s `applyEffectsFromBlocks` after its move: each
    /// cactus its box is in takes a point off its 5 health
    /// (`ExperienceOrb.hurtServer`, no event), unless its
    /// `isInvulnerableToBase` refuses `cactus` (an `Invulnerable` orb, or a
    /// mod's hook). True when that destroyed it.
    pub(super) fn orb_cactus_hits(&mut self, i: usize) -> bool {
        if self.remote.is_some() {
            return false;
        }
        let bounds = self.orbs[i].bounds();
        let hits = cells_named(self, bounds, "minecraft:cactus").len();
        let id = self.orbs[i].id;
        (0..hits).any(|_| {
            let source = horizon_client::health::HurtSource {
                damage_type: "minecraft:cactus",
                causing: 0,
                direct: 0,
                position: None,
            };
            let vanilla = self.saved_refuses_damage(id, "minecraft:cactus", false);
            let refused = crate::game::entity::damage_level_of!(self).invulnerability_check(
                horizon_client::health::Hurting::Entity(id as i32, "minecraft:experience_orb", source),
                vanilla,
            );
            !refused && self.orbs[i].hurt(1.0)
        })
    }
}

#[cfg(test)]
#[path = "tests/nonliving_hurt_tests.rs"]
mod tests;
