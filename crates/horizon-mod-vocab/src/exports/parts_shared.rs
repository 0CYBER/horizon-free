//! Exports of `crate::parts` items defined in `parts/shared.rs`.
pub use parts::{Attached, Parts, ProvidedBy};
pub(crate) use parts::{has_part_hook, parts, worldgen_parts};
mod __private {
    pub use crate::parts::{
        HookParts, KnownIds, PartsRef, ResolvePartsError, clear_game_parts, has_part_hook,
        has_scoped_hook, install_resolved_parts, mark_game_thread, parts, prepare_parts,
        publish_worldgen_parts, resolve_parts, worldgen_feature, worldgen_part_mods,
    };
}
