//! The `can_swim_in_fluid` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0358

pub trait Hooks {
    /// Edit player swim admission for the fluid being tested, including its source or flowing
    /// identity. `can_swim` starts true for water and false for other fluids. `fire_resistance`
    /// is the player's active Fire Resistance effect from the body being simulated.
    #[notes = r#"## Player swimming in fluids

- **`can_swim_in_fluid(context, entity, kind, fluid, fire_resistance, can_swim)`**: change which fluids a player can swim in. Admission affects the swim flag, sneak-to-dive, swim sprint and crawl slowdown. It does not replace the separate body, eye and feet immersion tests, grant fire immunity, or change fluid currents, sounds or breathing. The fluid key distinguishes source and flowing forms. Defaults to allowing water only. The `fire_resistance` input is the active effect on the simulated player, not a host query. Register on both sides and keep the listener pure: the same decision is made on replayed movement ticks. This hook does not run for mobs."#]
    #[hook(
        side = "both",
        contract = "listen",
        context = "loop",
        cancellable = false,
        version = 2,
        target = "horizon_client_lib::motion::native_can_swim_in_fluid",
        neoforge = "ILivingEntityExtension.canSwimInFluidType"
    )]
    fn can_swim_in_fluid();
}
