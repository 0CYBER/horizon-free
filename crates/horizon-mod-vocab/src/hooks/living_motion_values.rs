//! The `living_motion_values` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A living entity's jump power (`LivingEntity.getJumpPower`: jump strength attribute, block jump factor and Jump Boost) is about to be used by `jumpFromGround`. `entity` is its id and `kind` its type; a mod may change `power`. The rabbit's own hop and the player, cube, panda and ordinary mob jumps all pass through it. Fires on both sides, on the client's replayed ticks too, with the loop `context` of the side that ticks the entity (its registries, settings and mod list): register the mod on both, and keep the listener a pure function of its inputs so prediction and the server agree. Beyond NeoForge: there is no counterpart. Diverges from upstream: new hook.
    #[notes = r#"## Living motion values

- **`living_jump_power(context, entity, kind, power)`, `living_speed(context, entity, kind, speed)` and `living_flying_speed(context, entity, kind, speed)`**: edit a living entity's jump power (`LivingEntity.getJumpPower`), movement speed (`getSpeed`) and airborne acceleration (`getFlyingSpeed`) where its travel and jump read them. They cover the player and the mobs' ground, fluid and jump paths (the rabbit's hop included); a mob that ships its own flying travel (an allay, a guardian) still reads its own speed. They fire on both sides and on the client's replayed ticks, so a mod registers on both and keeps each listener a pure function of its inputs. `context` is the loop context of the side that ticks the entity (`context.side()`, registries, settings, mod list): the server's for a mob or a server player, the client's for its own prediction and replayed ticks. Contract version `2`: a mod written for `1` takes the context as a new first argument of the `Hooks` method. With no listener the cost is one `listened` branch per call. Beyond NeoForge (no counterpart); Diverges from upstream: new hooks. For per-block jump and speed factors see `block_jump_factor` and `block_speed_factor` below."#]
    #[hook(
        side = "both",
        contract = "listen",
        context = "loop",
        cancellable = false,
        version = 2,
        target = "horizon_client_lib::motion::native_living_jump_power",
        neoforge = "none (vanilla LivingEntity.getJumpPower; not LivingEvent.LivingJumpEvent)"
    )]
    fn living_jump_power();
    /// A living entity's movement speed (`LivingEntity.getSpeed`) is about to drive its travel on the ground and through fluids. `entity` is its id and `kind` its type; a mod may change `speed`. Fires on both sides, on the client's replayed ticks too, with the loop `context` of the side that ticks the entity (its registries, settings and mod list): register the mod on both, and keep the listener a pure function of its inputs so prediction and the server agree. Beyond NeoForge: there is no counterpart. Diverges from upstream: new hook.
    #[hook(
        side = "both",
        contract = "listen",
        context = "loop",
        cancellable = false,
        version = 2,
        target = "horizon_client_lib::motion::native_living_speed",
        neoforge = "none (vanilla LivingEntity.getSpeed; NeoForge adds no event or extension for it)"
    )]
    fn living_speed();
    /// A living entity's airborne acceleration (`LivingEntity.getFlyingSpeed`) is about to be used by its travel. `entity` is its id and `kind` its type; a mod may change `speed`. Fires on both sides, on the client's replayed ticks too, with the loop `context` of the side that ticks the entity (its registries, settings and mod list): register the mod on both, and keep the listener a pure function of its inputs so prediction and the server agree. Beyond NeoForge: there is no counterpart. Diverges from upstream: new hook.
    #[hook(
        side = "both",
        contract = "listen",
        context = "loop",
        cancellable = false,
        version = 2,
        target = "horizon_client_lib::motion::native_living_flying_speed",
        neoforge = "none (vanilla LivingEntity.getFlyingSpeed; NeoForge adds no event or extension for it)"
    )]
    fn living_flying_speed();
}
