//! Horizon's polar spyglass look: the yaw scale that keeps a scoped look's sideways screen
//! motion where it is at the horizon while looking up or down.
//!
//! A yaw turn of `d` degrees at pitch `p` moves the picture sideways by about `d·cos(p)` and
//! rolls it around the centre by about `d·sin(p)`, so vanilla's equal rates on both axes make
//! the horizontal look slow down as the pitch climbs. Turning the yaw `1/|cos(p)|` faster puts
//! the sideways motion back, and the roll is left as the only axis that grows with the pitch.
//!
//! The scale is held at [`MAX`] because `1/cos` is an infinity at the pole: with the cap the
//! scale stays finite, the roll it adds stays bounded, and a turn at the pole is still a
//! rotation. This is Horizon's own policy, so it lives in its own module of the Horizon layer;
//! the engine's mouse (`horizon_client_input::mouse`) only sees a plain
//! [`LookOptions::yaw_scale`](crate::mouse::LookOptions::yaw_scale).

/// The most the polar look multiplies a scoped yaw by: `1/|cos(pitch)|` reaches it at 82.82°.
pub const MAX: f64 = 8.0;

/// The yaw scale of a scoped look at `pitch` degrees (`Entity.xRot`): `1/|cos(pitch)|`, held
/// at [`MAX`]. 1.0 when the option is off, and for a pitch that isn't a number.
pub fn yaw_scale(pitch: f32, enabled: bool) -> f64 {
    if !enabled || !pitch.is_finite() {
        return 1.0;
    }
    let cos = f64::from(pitch).to_radians().cos().abs();
    (1.0 / cos).min(MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// On, the scale is `1/|cos(pitch)|`: the sidelong screen shift a yaw turn of `d` degrees
    /// makes is `d·cos(pitch)`, so this is how much faster the turn has to be to keep it.
    #[test]
    fn the_scale_is_one_over_cos_below_the_cap() {
        for (pitch, want) in [
            (0.0f32, 1.0),
            (45.0, std::f64::consts::SQRT_2),
            (60.0, 2.0),
            (80.0, 5.758770483143634),
        ] {
            assert!(want < MAX, "pitch {pitch} is below the cap: {want}");
            let got = yaw_scale(pitch, true);
            assert!(
                (got - want).abs() < 1e-9,
                "pitch {pitch}: {got} against {want}"
            );
        }
    }

    /// On, the scale stops at the cap instead of running to an infinity at the poles, and a
    /// pitch that isn't a number changes nothing.
    #[test]
    fn the_scale_is_finite_at_the_poles_and_for_a_pitch_that_is_not_a_number() {
        for pitch in [82.82f32, 89.0, 89.9, 90.0, -90.0] {
            assert_eq!(yaw_scale(pitch, true), MAX, "pitch {pitch}");
        }
        for pitch in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(yaw_scale(pitch, true), 1.0, "pitch {pitch}");
        }
    }

    /// Off, the scale is exactly 1.0: vanilla's turn, bit for bit, at every pitch.
    #[test]
    fn the_scale_is_one_when_the_option_is_off() {
        for pitch in [0.0f32, 45.0, 80.0, 89.0, 90.0, f32::NAN] {
            assert_eq!(yaw_scale(pitch, false), 1.0, "pitch {pitch}");
        }
    }
}
