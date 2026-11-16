//! The player's motion for mods (docs/modding/src/how-to/native-mods.md, "A player's motion"): a push for the local player, on a
//! client.
use super::Game;

/// Each component of a push is clamped to this, in blocks a tick.
const PUSH_MAX: f32 = 4.0;
/// The velocity after a push: `kick` added, each component clamped; none when one of `kick`'s isn't a number.
fn pushed(velocity: [f64; 3], kick: [f32; 3]) -> Option<[f64; 3]> {
    if !kick.iter().all(|v| v.is_finite()) {
        return None;
    }
    Some(std::array::from_fn(|i| {
        velocity[i] + f64::from(kick[i].clamp(-PUSH_MAX, PUSH_MAX))
    }))
}

#[allow(dead_code)]
impl Game<'_> {
    /// For a client mod: a push added to the local player's velocity, in blocks a tick, as an
    /// explosion's knockback is (the window's player moves itself, as vanilla's client does). Each component is
    /// clamped to -4..4; a spectator, or a value that is not a number, is left alone.
    pub(super) fn push_player(&mut self, kick: [f32; 3]) {
        if let Some(p) = self.player.as_mut()
            && !p.spectator
            && let Some([x, y, z]) = pushed([p.velocity.x, p.velocity.y, p.velocity.z], kick)
        {
            p.velocity = horizon_core::geometry::Vec3::new(x, y, z);
            if kick[1] > 0.0 {
                p.ground.on_ground = false;
            }
        }
    }
}

#[cfg(test)]
mod motion_tests {
    use super::*;

    #[test]
    fn a_push_adds_to_the_velocity_clamped() {
        assert_eq!(
            pushed([0.1, -0.08, 0.0], [0.8, 0.55, -0.5]),
            Some([0.1 + 0.8f32 as f64, -0.08 + 0.55f32 as f64, -0.5])
        );
        assert_eq!(pushed([0.0; 3], [9.0, -9.0, 0.0]), Some([4.0, -4.0, 0.0]));
        assert_eq!(pushed([1.0; 3], [f32::NAN, 0.0, 0.0]), None);
        assert_eq!(pushed([1.0; 3], [0.0, f32::INFINITY, 0.0]), None);
    }
}
