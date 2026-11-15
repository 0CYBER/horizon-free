//! One head listener that changes its argument, built into the wasm fixture and into a native
//! test, at the real point of `horizon_client::game::commands::wrap_degrees(angle: f32) -> f32`:
//! the body runs with the listener's argument on both targets.

/// Turns the angle half a circle before the body wraps it.
pub fn turn_angle(args: &mut mod_points::GameCommandsWrapDegreesArgs) {
    args.angle += 180.0;
}

/// Each call's angle and what `Mth.wrapDegrees` returns for the turned one.
pub const CASES: [(f32, f32); 2] = [(10.0, -170.0), (-90.0, 90.0)];
