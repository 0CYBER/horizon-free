//! `Level.tickBlockEntities` helpers: the ticker name a sample is attributed to and the
//! block-ticking test of a level.
use horizon_client::{containers::BlockEntity, fast_hash::FxHashSet, ticket_levels::Levels};

/// Shared decision for a borrowed position read and an owned tick-phase snapshot.
pub fn block_ticking_at(levels: &Levels, pending: &FxHashSet<(i32, i32)>, x: i32, z: i32) -> bool {
    levels.block_ticking_at(x, z) && !pending.contains(&(x >> 4, z >> 4))
}

/// The block entity type a sample is attributed to while its ticker runs.
pub fn ticker_name(entity: &BlockEntity) -> &'static str {
    match entity {
        BlockEntity::Chest(_) => "hopper",
        BlockEntity::Crafter(_) => "crafter",
        BlockEntity::Jukebox(_) => "jukebox",
        BlockEntity::Beacon(_) => "beacon",
        BlockEntity::CreakingHeart(_) => "creaking_heart",
        BlockEntity::Spawner(_) => "mob_spawner",
        BlockEntity::TrialSpawner(_) => "trial_spawner",
        BlockEntity::Vault(_) => "vault",
        BlockEntity::Beehive(_) => "beehive",
        BlockEntity::Furnace(_) => "furnace",
        BlockEntity::Brewing(_) => "brewing_stand",
        BlockEntity::DaylightDetector => "daylight_detector",
        BlockEntity::Campfire(_) => "campfire",
        BlockEntity::Mod(_) => "mod_block_entity",
        BlockEntity::PotentSulfur(_) => "potent_sulfur",
        _ => "block_entity",
    }
}
