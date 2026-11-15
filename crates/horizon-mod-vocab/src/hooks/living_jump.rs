//! The `living_jump` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0348

pub trait Hooks {
    /// A living entity jumped (`LivingEvent.LivingJumpEvent`). It is given the loop `context` of the side that ticks the entity (its registries, settings and mod list); it fires inside the tick's own jump, after the velocity write, so it takes no `Host`.
    #[hook(
        side = "both",
        contract = "listen",
        context = "loop",
        neoforge = "LivingEvent.LivingJumpEvent",
        version = 2,
        target = "horizon_client_lib::motion::native_living_jump"
    )]
    fn living_jump();
}
