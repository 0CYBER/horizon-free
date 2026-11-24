pub trait Hooks {
    /// Edit the player's active water or lava travel, before acceleration and the move. `deep`
    /// means body immersion exceeds the fluid jump threshold, not that the eyes are submerged.
    /// `fire_resistance` is the active Fire Resistance effect of the body being simulated.
    /// The travel lane keeps vanilla's water-before-lava precedence; where its body intersects
    /// multiple forms of that fluid, `fluid` names the form with the greatest tracked depth.
    #[notes = r#"## Player fluid travel

- **`living_fluid_travel`**: edit acceleration, horizontal and vertical drag, whether to run the fluid falling adjustment, extra gravity as a factor of effective gravity, and the optional vertical climb impulse before drag when colliding horizontally on a climbable block. The body immersion's fluid key distinguishes source and flowing forms. The water lane supplies acceleration and horizontal drag after sprint, water movement efficiency and Dolphin's Grace; vertical drag is vanilla `0.8f32` widened to `f64`, falling adjustment is enabled, extra gravity factor is zero, and climb impulse is `Some(0.2)`. The lava lane supplies acceleration `0.02`, horizontal drag `0.5`, deep/shallow vertical drag `0.5`/water's vertical drag, falling adjustment only in shallow lava, extra gravity factor `0.25`, and no climb impulse. The falling adjustment still respects vanilla's sprint and zero-gravity guards, and the separate ledge escape check still runs. Only the player's existing water/lava travel calls this hook; it does not add a travel lane for custom fluids or change mob travel. Register on both sides and keep the listener pure for replay."#]
    #[hook(
        side = "both",
        contract = "listen",
        context = "loop",
        cancellable = false,
        version = 2,
        target = "horizon_client_lib::motion::native_living_fluid_travel",
        neoforge = "FluidType.move (value edits, not replacement travel)"
    )]
    fn living_fluid_travel(
        context: &LoopContext,
        entity: u32,
        kind: &str,
        fluid: &str,
        deep: bool,
        fire_resistance: bool,
        acceleration: &mut f32,
        horizontal_drag: &mut f64,
        vertical_drag: &mut f64,
        falling_adjustment: &mut bool,
        extra_gravity_factor: &mut f64,
        climb_impulse: &mut Option<f64>,
    );
}
