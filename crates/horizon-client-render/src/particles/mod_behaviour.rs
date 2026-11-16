//! A mod particle's declared behaviour (`particles/<name>.json` of a native mod): how it moves, walks
//! through its sprite set and fades, read once when the sprite sets load.
use super::{Behaviour, Particle};
use horizon_core::random::{LegacyRandom, RandomSource};

/// How a mod particle walks through its sprite set.
#[derive(Clone, Copy, Debug, PartialEq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
enum ModAnimation {
    /// Over its lifetime, first to last sprite.
    Age,
    /// Cycles the sprites every `frame_ticks`.
    Loop,
    /// One random sprite, kept (what a particle without `animation` does).
    Random,
    /// The first sprite, kept.
    First,
}
#[derive(Clone, Copy, Debug, Default, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct ModBehaviour {
    velocity: Option<[f64; 3]>,
    /// Random extra velocity, each axis uniform in `-jitter..=jitter`.
    velocity_jitter: Option<[f64; 3]>,
    animation: Option<ModAnimation>,
    /// Ticks per frame of the `loop` animation.
    frame_ticks: Option<u16>,
    gravity: Option<f32>,
    friction: Option<f32>,
    lifetime: Option<i32>,
    size: Option<[f32; 2]>,
    alpha: Option<[f32; 2]>,
}
impl ModBehaviour {
    /// Whether the particle begins on the first sprite of its set instead of a random one.
    pub(super) fn starts_on_first_sprite(&self) -> bool {
        matches!(
            self.animation,
            Some(ModAnimation::First | ModAnimation::Age | ModAnimation::Loop)
        )
    }
    pub(super) fn parse(json: &serde_json::Value) -> Option<Self> {
        let behaviour: Self = serde_json::from_value(json.clone()).ok()?;
        if behaviour
            .velocity
            .is_some_and(|v| v.iter().any(|c| !c.is_finite() || c.abs() > 16.0))
            || behaviour
                .velocity_jitter
                .is_some_and(|v| v.iter().any(|c| !(0.0..=16.0).contains(c)))
            || behaviour.frame_ticks.is_some_and(|v| {
                !(1..=200).contains(&v) || behaviour.animation != Some(ModAnimation::Loop)
            })
            || behaviour
                .gravity
                .is_some_and(|v| !v.is_finite() || v.abs() > 100.0)
            || behaviour
                .friction
                .is_some_and(|v| !(0.0..=1.0).contains(&v))
            || behaviour
                .lifetime
                .is_some_and(|v| !(1..=12000).contains(&v))
            || behaviour
                .size
                .is_some_and(|v| v.iter().any(|c| !(0.0..=16.0).contains(c)))
            || behaviour
                .alpha
                .is_some_and(|v| v.iter().any(|c| !(0.0..=1.0).contains(c)))
        {
            return None;
        }
        Some(behaviour)
    }

    /// `spawn` is the velocity the spawn call passed; a declared velocity is added to it,
    /// with the jitter drawn from `random`.
    pub(super) fn apply(self, particle: &mut Particle, spawn: [f64; 3], random: &mut LegacyRandom) {
        if self.velocity.is_some() || self.velocity_jitter.is_some() {
            let base = self.velocity.unwrap_or([0.0; 3]);
            let jitter = self.velocity_jitter.unwrap_or([0.0; 3]);
            for axis in 0..3 {
                let spread = if jitter[axis] > 0.0 {
                    f64::from(random.next_float() * 2.0 - 1.0) * jitter[axis]
                } else {
                    0.0
                };
                particle.velocity[axis] = base[axis] + spawn[axis] + spread;
            }
        }
        if let Some(gravity) = self.gravity {
            particle.gravity = gravity;
        }
        if let Some(friction) = self.friction {
            particle.friction = friction;
        }
        if let Some(lifetime) = self.lifetime {
            particle.lifetime = lifetime;
        }
        match self.animation {
            Some(ModAnimation::Loop) => particle.frame_ticks = self.frame_ticks.unwrap_or(2),
            Some(ModAnimation::Age) => {}
            // No `animation`, `random` or `first`: the sprite chosen at spawn is kept.
            Some(ModAnimation::Random | ModAnimation::First) | None => particle.set = None,
        }
        if self.size.is_some() || self.alpha.is_some() {
            let size = self.size.unwrap_or([particle.size; 2]);
            let alpha = self.alpha.unwrap_or([particle.rgba[3]; 2]);
            particle.size = size[0];
            particle.rgba[3] = alpha[0];
            particle.behaviour = Behaviour::Mod { size, alpha };
        } else if matches!(self.animation, Some(ModAnimation::Age | ModAnimation::Loop)) {
            // The tick that walks the sprite set (base tick, then `sprite_from_age`).
            particle.behaviour = Behaviour::Animated;
        }
    }
}

pub(super) fn mod_curve([start, end]: [f32; 2], age: i32, lifetime: i32) -> f32 {
    let progress = (age as f32 / lifetime.max(1) as f32).clamp(0.0, 1.0);
    start + (end - start) * progress
}
