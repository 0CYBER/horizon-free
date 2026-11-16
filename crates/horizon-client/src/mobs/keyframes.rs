//! Vanilla keyframe animations (`AnimationDefinition`, `AnimationChannel`,
//! `KeyframeAnimation`) exported from the client's animation definitions,
//! applied to a pose map exactly like `KeyframeAnimation.Entry.apply`.
use mod_api::{AnimationChannel, AnimationTarget, Interpolation, Keyframe};

#[cfg(test)]
#[path = "keyframes_parity_tests.rs"]
mod parity_tests;
use serde_json::Value;

/// A vanilla animation as the game exports it: channels and their sampling are `mod_api`'s, the
/// one code path the game and a mod's animations share.
#[derive(Clone, Debug)]
pub struct Animation {
    pub length: f32,
    pub looping: bool,
    pub bones: Vec<(String, Vec<AnimationChannel>)>,
}
fn v3(v: &Value) -> [f32; 3] {
    let a = |i: usize| v[i].as_f64().unwrap_or(0.0) as f32;
    [a(0), a(1), a(2)]
}
impl Animation {
    pub fn parse(v: &Value) -> Self {
        let bones = v["bones"]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(name, channels)| {
                let channels = channels
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|c| AnimationChannel {
                        target: match c["target"].as_str() {
                            Some("position") => AnimationTarget::Position,
                            Some("scale") => AnimationTarget::Scale,
                            _ => AnimationTarget::Rotation,
                        },
                        keyframes: c["keyframes"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .map(|k| Keyframe {
                                time: k[0].as_f64().unwrap_or(0.0) as f32,
                                pre: v3(&k[1]),
                                post: v3(&k[2]),
                                interpolation: if k[3].as_str() == Some("catmullrom") {
                                    Interpolation::CatmullRom
                                } else {
                                    Interpolation::Linear
                                },
                            })
                            .collect(),
                    })
                    .collect();
                (name.clone(), channels)
            })
            .collect();
        Self {
            length: v["length"].as_f64().unwrap_or(1.0) as f32,
            looping: v["looping"].as_bool().unwrap_or(false),
            bones,
        }
    }
    /// KeyframeAnimation.apply(millis, scale): offsets added to each bone's pose.
    pub fn apply(&self, millis: i64, scale: f32, poses: &mut super::anim::Poses) {
        let mut seconds = millis as f32 / 1000.0;
        if self.looping && self.length > 0.0 {
            seconds %= self.length;
        }
        for (bone, channels) in &self.bones {
            for c in channels {
                // `parse` above builds only built-in interpolations, which always sample.
                let Some(v) = c.sample(seconds, scale) else {
                    continue;
                };
                let p = poses.get(bone);
                match c.target {
                    // ModelPart.offsetPos / offsetRotation / offsetScale.
                    AnimationTarget::Position => {
                        p[0] += v[0];
                        p[1] += v[1];
                        p[2] += v[2];
                    }
                    AnimationTarget::Rotation => {
                        p[3] += v[0];
                        p[4] += v[1];
                        p[5] += v[2];
                    }
                    AnimationTarget::Scale => {
                        p[6] += v[0];
                        p[7] += v[1];
                        p[8] += v[2];
                    }
                    // `parse` above builds only the three built-in targets.
                    AnimationTarget::Registered(_) => {}
                }
            }
        }
    }
    /// KeyframeAnimation.applyWalk(pos, speed, speedFactor, scaleFactor).
    pub fn apply_walk(
        &self,
        pos: f32,
        speed: f32,
        speed_factor: f32,
        scale_factor: f32,
        poses: &mut super::anim::Poses,
    ) {
        let time = (pos * 50.0 * speed_factor) as i64;
        let scale = (speed * scale_factor).min(1.0);
        self.apply(time, scale, poses);
    }
}
/// AnimationState: a start time in ticks while started.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AnimationState {
    pub started: Option<f32>,
}
impl AnimationState {
    pub fn start(&mut self, age: f32) {
        self.started = Some(age);
    }
    pub fn start_if_stopped(&mut self, age: f32) {
        if self.started.is_none() {
            self.started = Some(age);
        }
    }
    pub fn stop(&mut self) {
        self.started = None;
    }
    /// `AnimationState.fastForward`: the animation is `ticks` further on.
    pub fn fast_forward(&mut self, ticks: i32) {
        if let Some(s) = &mut self.started {
            *s -= ticks as f32;
        }
    }
    pub fn animate_when(&mut self, condition: bool, age: f32) {
        if condition {
            self.start_if_stopped(age)
        } else {
            self.stop()
        }
    }
    /// Milliseconds since the start at render time `age` (ticks + partial).
    pub fn millis(&self, age: f32) -> Option<i64> {
        self.started.map(|s| ((age - s) * 50.0) as i64)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn looping_animation_applies_walk_without_overflow() {
        let anim = Animation {
            length: 1.25,
            looping: true,
            bones: vec![(
                "body".to_string(),
                vec![AnimationChannel {
                    target: AnimationTarget::Position,
                    keyframes: vec![
                        Keyframe {
                            time: 0.0,
                            pre: [0.0; 3],
                            post: [0.0, 1.0, 0.0],
                            interpolation: Interpolation::Linear,
                        },
                        Keyframe {
                            time: 1.25,
                            pre: [0.0, 1.0, 0.0],
                            post: [0.0, 1.0, 0.0],
                            interpolation: Interpolation::Linear,
                        },
                    ],
                }],
            )],
        };
        let mut poses = super::super::anim::Poses::default();
        anim.apply_walk(50.0, 1.0, 1.0, 2.5, &mut poses);
        assert!((poses.get("body")[1] - 1.0).abs() < 1e-6);
    }
}
