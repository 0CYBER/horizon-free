//! The `makes_open_trapdoor_above_climbable` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Whether an open trapdoor above the block state this part is attached to climbs like a ladder (`IBlockExtension.makesOpenTrapdoorAboveClimbable`, which `LivingEntity.onClimbable` asks of the block under the open trapdoor a living entity stands in). `state` is that block's state and `pos` its position; `trapdoor` is the open trapdoor's state above it. `climbable` starts with vanilla's answer, true for a `minecraft:ladder` facing the trapdoor's way, and a part may set either value. Asked only once the trapdoor itself was not a ladder (`is_ladder` and the `#minecraft:climbable` tag), so it never makes a block that is a ladder stop climbing. A gliding entity in a `#minecraft:can_glide_through` block, a spectator and a player who is flying are never asked. Read for the player, the mobs, and the climbing a mod reads through `LivingRef::is_climbing`. It fires on both sides and on the client's replayed ticks, so register the mod on both and keep the listener a pure function of its inputs. It is given the loop `context` of the side that ticks the entity, and takes no `Host`. With no part attached to the state below, the owning game checks the hook subscription and prepared API block key before any event conversion, so the cost is one table lookup. Armor stands never ask it. Diverges from upstream: new hook.
    #[hook(
        side = "both",
        contract = "listen",
        context = "loop",
        scope = "block",
        key = "state",
        cancellable = false,
        target = "horizon_client_lib::motion::ladder::native_makes_open_trapdoor_above_climbable",
        neoforge = "IBlockExtension.makesOpenTrapdoorAboveClimbable"
    )]
    fn makes_open_trapdoor_above_climbable();
}
