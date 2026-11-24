//! The `hurt_camera` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0356

pub trait Hooks {
    /// Before `GameRenderer.bobHurt` draws a damage or death frame, once for both the world
    /// and the first-person hand. `partial_tick` is the camera's tick fraction. `strength`
    /// starts at the player's Damage Tilt setting; multiply it to scale the damage tilt.
    /// `hurt_dir` starts at vanilla's yaw axis in degrees; set it to 0 for a straight roll.
    /// Changed directions wrap within a turn. Non-finite values, or a strength too large
    /// for the renderer's float angle, leave the corresponding vanilla value in place.
    /// Cancel to skip all of `bobHurt`, including the death twist. Strength and direction
    /// edits alone do not change the death twist. View Bobbing, `camera_shake`, and gameplay
    /// damage are unaffected.
    #[hook(
        side = "client",
        contract = "cancel",
        out = "canceled",
        neoforge = "none (vanilla GameRenderer.bobHurt)",
        cancellable = true
    )]
    #[notes = r##"## Hurt camera

`hurt_camera` changes vanilla `GameRenderer.bobHurt`, not the additive `camera_shake` hook. The first-person hand and the world share one result each damage or death frame. Multiply `strength` to scale the Damage Tilt option, or set `hurt_dir` to 0 for the old, straight-on roll. Return `Flow::Cancel` or set `cancelled` to skip both the damage tilt and the death twist; setting strength to 0 leaves the death twist in place. Walking bob and mod-added shake still draw, and nothing changes the hit itself."##]
    fn hurt_camera(partial_tick: f32, strength: &mut f64, hurt_dir: &mut f32, canceled: &mut bool);
}
