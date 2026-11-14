//! `LivingEntity.onClimbable` for mobs: the block a mob stands in, and the mods' say on whether it
//! is a ladder (`native_is_ladder`) and on whether the block under an open trapdoor makes the
//! trapdoor one (`native_makes_open_trapdoor_above_climbable`).
use crate::mobs::{Mob, MobWorld};
use horizon_core::position::BlockPos;

impl Mob {
    /// `LivingEntity.onClimbable` (`Spider.onClimbable`: the climbing flag):
    /// the block it stands in is `#minecraft:climbable` (vines, ladders,
    /// scaffolding...) or its mod parts' `is_ladder` says it is, or it is an open trapdoor over
    /// a ladder facing the same way, as the parts of the block below may decide
    /// (`trapdoorUsableAsLadder`). A mob gliding (`isFallFlying`) through a
    /// `#minecraft:can_glide_through` block (vines) is not on it: that test
    /// comes first.
    pub fn on_climbable(&self, w: &MobWorld) -> bool {
        if self.kind.flags.climber {
            return self.climbing;
        }
        // `AbstractHorse.onClimbable`, `Ghast.onClimbable`, `Phantom.onClimbable`
        // and `HappyGhast.onClimbable` are a plain false: no horse, donkey,
        // mule, undead horse, llama, trader llama, camel or camel husk (nor
        // those flying monsters) is ever on a climbable block, whatever it
        // stands in.
        if crate::mobs::equine::is_equine(self.species())
            || matches!(
                self.species(),
                crate::mobs::Species::Ghast
                    | crate::mobs::Species::Phantom
                    | crate::mobs::Species::HappyGhast
            )
        {
            return false;
        }
        let (x, y, z) = (
            self.position.x.floor() as i32,
            self.position.y.floor() as i32,
            self.position.z.floor() as i32,
        );
        let state = w.world.block(x, y, z);
        let facts = w.facts.get(state);
        if self.extra.fall_flying && w.has_tag("minecraft:can_glide_through", &facts.name) {
            return false;
        }
        // `IBlockExtension.isLadder`: the block's parts answer after the tag.
        let mut ladder = w.has_tag("minecraft:climbable", &facts.name);
        crate::motion::native_is_ladder(
            w.loop_context,
            self.id,
            BlockPos::new(x, y, z),
            state,
            &mut ladder,
        );
        if ladder {
            return true;
        }
        if !facts.has_class("TrapDoorBlock") || w.prop(x, y, z, "open") != Some("true") {
            return false;
        }
        // `IBlockExtension.makesOpenTrapdoorAboveClimbable`: the parts of the block below answer
        // after vanilla's ladder test.
        let mut climbable = w.block_name(x, y - 1, z) == "minecraft:ladder"
            && w.prop(x, y - 1, z, "facing") == w.prop(x, y, z, "facing");
        crate::motion::ladder::native_makes_open_trapdoor_above_climbable(
            w.loop_context,
            w.world.block(x, y - 1, z),
            BlockPos::new(x, y - 1, z),
            state,
            &mut climbable,
        );
        climbable
    }
}
