//! The `is_ladder` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Whether a living entity climbs the block state this part is attached to (`IBlockExtension.isLadder`, which `LivingEntity.onClimbable` asks of the block the entity stands in). `entity` is the climber's id and `pos` the block it stands in; `state` is that block's state. `ladder` starts with vanilla's answer, whether the block is in `#minecraft:climbable`, and a part may set either value: `true` makes the block climbable as a ladder is, `false` refuses it though the tag lists it. A gliding entity (`isFallFlying`) in a `#minecraft:can_glide_through` block, a spectator and a player who is flying are never asked, as vanilla never climbs for them, and an open trapdoor over a ladder climbs as before. Read for the player, the mobs, and the climbing a mod reads through `LivingRef::is_climbing`. It fires on both sides and on the client's replayed ticks, so register the mod on both and keep the listener a pure function of its inputs. It is given the loop `context` of the side that ticks the entity, and takes no `Host`. With no part attached to the state, the owning game checks the hook subscription and prepared API block key before any event conversion, so the cost is one table lookup. Armor stands never ask it. Diverges from upstream: new hook.
    #[hook(
        side = "both",
        contract = "listen",
        context = "loop",
        scope = "block",
        key = "state",
        cancellable = false,
        target = "horizon_client_lib::motion::surface::native_is_ladder",
        neoforge = "IBlockExtension.isLadder"
    )]
    fn is_ladder();
}
