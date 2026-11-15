/// One tick of a bot's keys for `native_simulate_player_move`, as a client's input is
/// (`ClientInput`, `LocalPlayer.aiStep`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct PlayerInput {
    /// Forward (1) or back (-1); in between is a stick's partial push.
    pub forward: f32,
    /// Left (1) or right (-1), as vanilla's `leftImpulse`.
    pub strafe: f32,
    /// Jump is held.
    pub jump: bool,
    /// Sneak is held.
    pub sneak: bool,
    /// Sprint is held; it sprints only while moving forward.
    pub sprint: bool,
    /// Where the body faces, in degrees (`Entity.getYRot`, `getXRot`).
    pub yaw: f32,
    /// Vertical look angle in degrees.
    pub pitch: f32,
    /// The velocity to start from (the last call's answer), in blocks a tick; None takes the
    /// server's copy of the player.
    pub velocity: Option<[f64; 3]>,
}

/// Where a bot's tick took it (`native_simulate_player_move`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct SimulatedMove {
    /// Feet position after the simulated tick.
    pub position: [f64; 3],
    /// The velocity after the tick, to pass back as the next tick's `velocity`.
    pub velocity: [f64; 3],
    /// Whether the resulting body rests on ground.
    pub on_ground: bool,
    /// It ran into a wall (`Entity.horizontalCollision`).
    pub horizontal_collision: bool,
    /// Whether it sprints after the tick (a wall or sneaking stops it).
    pub sprinting: bool,
}
