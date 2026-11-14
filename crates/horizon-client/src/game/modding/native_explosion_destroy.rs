//! Destroying a mod's own list of blocks as a blast would (`LevelRef::explosion_destroy_blocks`): the
//! explosion's hooks and its block step, without its rays, its entity damage, its packet or its
//! sound. A mod that casts its own rays breaks what they found with it, spread over ticks.
use super::Game;
use horizon_client::mobs::DamageKind;
use mod_api::{EntityRef, ExplosionDestroyError, ExplosionInteraction, LevelRef};
use std::collections::HashSet;

/// Below this radius the reference tells no detonate listener (`ServerExplosion.explode`).
const DETONATE_RADIUS: f32 = 1.0e-5;

impl Game<'_> {
    /// Destroys the blocks at `positions` as the blast `level.explode(source, center, radius, fire,
    /// interaction)` would: `explosion_start` (which may cancel it), `explosion_detonate` with the
    /// positions, then the blast's block step on those left. Positions in chunks that aren't
    /// loaded, or outside the level's height, are skipped and not counted. The number of blocks
    /// destroyed.
    pub(super) fn level_explosion_destroy_blocks(
        &mut self,
        level: LevelRef,
        positions: &[[i32; 3]],
        source: Option<EntityRef>,
        center: [f64; 3],
        radius: f32,
        fire: bool,
        interaction: ExplosionInteraction,
    ) -> Result<u32, ExplosionDestroyError> {
        if positions.len() as u64 > ExplosionDestroyError::MAX_POSITIONS {
            return Err(ExplosionDestroyError::TooMany(positions.len() as u64));
        }
        if !(center.iter().all(|v| v.is_finite()) && radius.is_finite() && radius >= 0.0) {
            return Err(ExplosionDestroyError::InvalidBlast);
        }
        if !self.level_in_place(level)
            || (self.serving.is_none() && self.options.dedicated.is_none())
        {
            return Err(ExplosionDestroyError::Unavailable);
        }
        let source = match source {
            Some(entity) => Some(
                self.native_explosion_source(entity)
                    .ok_or(ExplosionDestroyError::Unavailable)?,
            ),
            None => None,
        };
        let id = source.map(|(id, _)| id);
        let interaction = self.native_blast_interaction(interaction, id);
        // The blast's source and cause, as `native_explode` sets them, and the enclosing
        // blast's put back after.
        let outer = self.take_blast_state();
        if let Some((id, target)) = source {
            self.attribute_native_blast(id, target);
        }
        let destroyed = self.destroy_listed_blocks(positions, center, radius, fire, interaction);
        self.restore_blast_state(outer);
        Ok(destroyed)
    }

    /// The hooks and the block step of `level_explosion_destroy_blocks`, with the blast's state
    /// in place.
    fn destroy_listed_blocks(
        &mut self,
        positions: &[[i32; 3]],
        center: [f64; 3],
        radius: f32,
        fire: bool,
        interaction: super::explosion::BlockInteraction,
    ) -> u32 {
        let damage_type =
            super::creatures::damage_type(DamageKind::Explosion, self.blast_attacker().is_some());
        let start = self.explosion_start(center, radius, interaction, fire, damage_type);
        if self.explosion_start_cancelled(&start) {
            return 0;
        }
        // The positions in the order given, a repeated one once.
        let mut seen = HashSet::with_capacity(positions.len());
        let mut targets: Vec<[i32; 3]> = positions
            .iter()
            .copied()
            .filter(|p| seen.insert(*p))
            .collect();
        if radius >= DETONATE_RADIUS {
            // A listener only spares: what it takes out stays, what it adds is ignored.
            let mut told = targets.clone();
            // This block-only phase has no entity victims.
            self.native_explosion_detonate(self.dim.id(), center, &mut told, &mut Vec::new());
            let kept: HashSet<[i32; 3]> = told.into_iter().collect();
            targets.retain(|p| kept.contains(p));
        }
        // Unlike the reference's rays, a mod's list never loads terrain.
        let (min_y, max_y) = (self.world.bottom_y(), self.world.top_y());
        targets.retain(|p| {
            (min_y..max_y).contains(&p[1])
                && self.world.chunks.contains_key(&(p[0] >> 4, p[2] >> 4))
        });
        match self.explosion_block_step(&mut targets, radius, interaction, fire, &start) {
            Ok(hit) => hit as u32,
            Err(error) => {
                log::warn!("explosion_destroy_blocks: {error}");
                0
            }
        }
    }
}
