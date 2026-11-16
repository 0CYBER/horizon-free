//! Block callbacks shared by thrown projectiles and fishing bobbers.
use super::{Game, creatures::ProjectileHitter};
use horizon_client::{Result, mobs::Who};
use horizon_core::{geometry::Vec3, position::BlockPos};

impl Game<'_> {
    /// The common block-state callbacks before projectile-specific breaking.
    pub(super) fn projectile_block_impact(
        &mut self,
        projectile: &ProjectileHitter,
        pos: [i32; 3],
        at: Vec3,
        face: [i32; 3],
    ) -> Result<()> {
        self.bell_projectile_hit(projectile, pos, at, face);
        self.amethyst_projectile_hit(pos);
        if self.with_level(|blocks, level, ticks| {
            blocks.big_dripleaf_projectile_hit(level, ticks, BlockPos::new(pos[0], pos[1], pos[2]))
        }) {
            self.process_level_events()?;
        }
        Ok(())
    }

    /// `TargetBlock.onProjectileHit`: a target block takes its power from
    /// where the projectile landed and resets after 20 ticks for an arrow, 8
    /// for any other projectile. A player's shot counts as a target hit.
    pub(super) fn target_block_hit(
        &mut self,
        p: &ProjectileHitter,
        pos: [i32; 3],
        at: Vec3,
        face: [i32; 3],
    ) {
        let block = BlockPos::new(pos[0], pos[1], pos[2]);
        let arrow = p.arrow;
        let location = [at.x, at.y, at.z];
        let hit = self.with_level(|b, level, ticks| {
            b.target_projectile_hit(level, ticks, block, location, face, arrow)
        });
        if let Some(strength) = hit {
            let _ = self.process_level_events();
            self.native_target_hit(self.dim.id(), pos, strength);
            if p.owner == Some(Who::Player) && (p.guest.is_none() || p.guest == self.lan_acting) {
                self.award_custom_stat("target_hit", 1);
                let projectile = horizon_client::advancements::EntityView {
                    kind: p.kind.to_owned(),
                    position: [p.position.x, p.position.y, p.position.z],
                    ..Default::default()
                };
                self.trigger_target_hit(&projectile, strength);
            }
        }
    }
}
