//! What `/summon` and `/execute summon` share: `SummonCommand.createEntity`'s
//! checks, and the entity `ExecuteCommand.spawnEntityAndRedirect` makes.
use super::{Game, R, Target, err};

/// `ResourceArgument.getSummonableEntityType`: `EntityType.canSummon` is false
/// only for the two `noSummon()` types (`EntityTypes.PLAYER`, `FISHING_BOBBER`).
pub(super) fn summonable(kind: &str) -> R<()> {
    if matches!(kind, "minecraft:player" | "minecraft:fishing_bobber") {
        return Err(err("entity.not_summonable", &[&kind]));
    }
    Ok(())
}

impl Game<'_> {
    /// `SummonCommand.createEntity` before it loads the entity:
    /// `Level.isInSpawnableBounds(BlockPos.containing(pos))`, then the
    /// Peaceful refusal (`EntityType.isAllowedInPeaceful`).
    pub(super) fn summon_checks(&self, kind: &str, pos: [f64; 3]) -> R<()> {
        let block_y = pos[1].floor() as i32;
        if !(-30_000_000.0..30_000_000.0).contains(&pos[0])
            || !(-20_000_000..20_000_000).contains(&block_y)
            || !(-30_000_000.0..30_000_000.0).contains(&pos[2])
        {
            return Err(err("commands.summon.invalidPosition", &[]));
        }
        if self.ui.difficulty == 0 && !horizon_client::mobs::finalize::allowed_in_peaceful(kind) {
            return Err(err("commands.summon.failed.peaceful", &[]));
        }
        Ok(())
    }
    /// `ExecuteCommand.spawnEntityAndRedirect`: `createEntity(source, type,
    /// source.getPosition(), {}, finalize = true)`. A mob is finalized
    /// (`Mob.finalizeSpawn(.., COMMAND, null)`) before it joins, as `/summon`
    /// without NBT does; any other entity is what its empty compound loads.
    pub(super) fn summon_for_execute(&mut self, kind: &str, pos: [f64; 3]) -> R<Target> {
        summonable(kind)?;
        self.summon_checks(kind, pos)?;
        if self.res.mob_types().contains_key(kind) {
            let short = kind.trim_start_matches("minecraft:");
            // Loaded from `{id:..}`: the compound's yaw, 0.
            let id = self
                .spawn_mob_facing(
                    short,
                    pos,
                    false,
                    super::super::creatures::SpawnSite::Finalizing(
                        horizon_client::mobs::spawn_rules::Reason::Command,
                    ),
                    0.0,
                )
                // tryAddFreshEntityWithPassengers refused: ERROR_DUPLICATE_UUID.
                .ok_or_else(|| err("commands.summon.failed.uuid", &[]))?;
            return Ok(Target::Mob(id));
        }
        self.summon_with_nbt(kind, pos, "{}")?
            .ok_or_else(|| err("commands.summon.failed", &[]))
    }
}

#[cfg(test)]
#[path = "summon_execute_tests.rs"]
mod tests;
