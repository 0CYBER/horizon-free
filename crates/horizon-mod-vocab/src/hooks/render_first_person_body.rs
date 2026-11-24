//! The local first-person player body, before admission to the world entity pass.

pub trait Hooks {
    /// Decide whether the local player has a world-view body in plain first person. `player`
    /// is the interpolated render state (including its stance), and `riding` says whether the
    /// vanilla model is seated on a vehicle. Not asked in third person, spectator, free camera,
    /// sleep, or death. Starts with `body.show == false` each frame. If enabled, the ordinary
    /// player and living pre/post hooks wrap this draw, with their normal cancellation rules.
    /// The usual first-person hand pass still runs. See `FirstPersonBody` for offsets and filters.
    #[hook(side = "client", contract = "listen")]
    fn render_first_person_body();
}
