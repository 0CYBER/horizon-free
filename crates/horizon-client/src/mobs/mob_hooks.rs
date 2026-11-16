//! `MobHooks`: the native mod hooks the mob AI asks while it decides, and the
//! `MobWorld` helpers that ask them.
use super::{Mob, MobWorld, PlayerView, TargetChange, Who, path::PathType};
use horizon_world::registry::BlockStateId;

/// Native mod hooks the mob AI asks while it decides.
pub trait MobHooks {
    /// Positional block classification, before air and all vanilla rules.
    /// None declines to vanilla; the host reads the same owning level.
    fn block_path_type(&self, _state: BlockStateId, _pos: [i32; 3]) -> Option<PathType> {
        None
    }
    /// The neighbour's own classified type and position. A present answer ends
    /// the scan even when equal to the node's type; None keeps fluid/vanilla fallback.
    fn adjacent_block_path_type(
        &self,
        _state: BlockStateId,
        _pos: [i32; 3],
        _path_type: PathType,
        default: Option<PathType>,
    ) -> Option<PathType> {
        default
    }
    /// `EnderManAngerEvent`: whether a mod keeps enderman `enderman` calm
    /// though `player` stares at it.
    fn enderman_anger_suppressed(&self, enderman: u32, player: &PlayerView) -> bool;
    /// `EntityMobGriefingEvent`: whether mob `mob` may grief, `rule` (the
    /// `mob_griefing` game rule) unless a mod says otherwise.
    fn can_grief(&self, mob: u32, rule: bool) -> bool;
    /// Whether `can_grief` may differ from the rule (a mod listens), so a
    /// scan that griefing off would skip still has to run.
    fn grief_hooked(&self) -> bool;
    /// `LivingVisibilityEvent`: `visibility`, the player's visibility
    /// percent as mob `looking` targets it, as mods scale it.
    fn living_visibility(&self, player: &PlayerView, looking: u32, visibility: f64) -> f64;
    /// `LivingVisibilityEvent` for a mob target `target`: its visibility
    /// percent as mob `looking` targets it, as mods scale it.
    fn living_visibility_mob(&self, _target: u32, _looking: u32, visibility: f64) -> f64 {
        visibility
    }
    /// A proposed target change. A listener may cancel or substitute a target.
    fn living_change_target(
        &self,
        _mob: &mut Mob,
        _world: &MobWorld<'_>,
        _is_brain: bool,
        target: Option<Who>,
    ) -> TargetChange {
        TargetChange::Set(target)
    }
    /// `IBlockExtension.canEntityDestroy`: whether mob `mob` may destroy the
    /// block at `pos`, `vanilla` (the mob's own rule) unless the block's parts
    /// answer otherwise. Asked after the griefing gate and before
    /// `LivingDestroyBlockEvent`.
    fn can_entity_destroy(&self, _mob: u32, _pos: [i32; 3], vanilla: bool) -> bool {
        vanilla
    }
}
impl MobWorld<'_> {
    /// `IBlockExtension.canEntityDestroy` of the block at `pos` for mob `mob`:
    /// `vanilla`, the mob's own rule (`WitherBoss.canDestroy`), or what the
    /// block's parts answer (`native_can_entity_destroy`).
    pub fn can_entity_destroy(&self, mob: u32, pos: [i32; 3], vanilla: bool) -> bool {
        match self.mob_hooks {
            Some(hooks) => hooks.can_entity_destroy(mob, pos, vanilla),
            None => vanilla,
        }
    }
}
