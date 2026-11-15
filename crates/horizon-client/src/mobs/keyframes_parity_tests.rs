//! The game's own animations sample exactly as they did before the sampling moved into `mod_api`
//! (arismoko/horizon#2607): every exported vanilla definition, every channel, at a grid of times and scales,
//! bit for bit against the pre-move sampler kept here as the oracle.
use super::*;

/// The sampler as `keyframes.rs` had it before the move, unchanged but for reading the
/// interpolation from the keyframe's enum instead of a `catmull` flag.
fn oracle(k: &[Keyframe], seconds: f32, scale: f32) -> [f32; 3] {
    fn catmullrom(t: f32, p0: f32, p1: f32, p2: f32, p3: f32) -> f32 {
        0.5 * (2.0 * p1
            + (p2 - p0) * t
            + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t * t
            + (3.0 * p1 - p0 - 3.0 * p2 + p3) * t * t * t)
    }
    if k.is_empty() {
        return [0.0; 3];
    }
    let first = k.iter().position(|f| seconds <= f.time).unwrap_or(k.len());
    let prev = first.saturating_sub(1);
    let next = (prev + 1).min(k.len() - 1);
    let delta = seconds - k[prev].time;
    let alpha = if next != prev {
        (delta / (k[next].time - k[prev].time)).clamp(0.0, 1.0)
    } else {
        0.0
    };
    if k[next].interpolation == Interpolation::CatmullRom {
        let p0 = k[prev.saturating_sub(1)].post;
        let p1 = k[prev].post;
        let p2 = k[next].post;
        let p3 = k[(next + 1).min(k.len() - 1)].post;
        std::array::from_fn(|i| catmullrom(alpha, p0[i], p1[i], p2[i], p3[i]) * scale)
    } else {
        let a = k[prev].post;
        let b = k[next].pre;
        std::array::from_fn(|i| (a[i] + (b[i] - a[i]) * alpha) * scale)
    }
}

#[test]
#[ignore = "daily"]
fn every_vanilla_animation_samples_as_before_the_move() {
    let root = std::path::PathBuf::from(
        std::env::var_os("HORIZON_TEST_RESOURCES").expect("set HORIZON_TEST_RESOURCES"),
    );
    let animations = super::super::data::MobData::load_animations(&root);
    assert!(animations.len() >= 70, "{} definitions", animations.len());
    let mut samples = 0;
    for (name, animation) in &animations {
        for (bone, channels) in &animation.bones {
            for channel in channels {
                // Every 1/64 s from before the start to past the end, and a few scales.
                let steps = ((animation.length + 0.5) * 64.0) as i32;
                for step in -8..=steps {
                    let seconds = step as f32 / 64.0;
                    for scale in [1.0, 0.37, 2.5] {
                        let moved = channel
                            .sample(seconds, scale)
                            .expect("vanilla names only built-in interpolations")
                            .map(f32::to_bits);
                        let before = oracle(&channel.keyframes, seconds, scale).map(f32::to_bits);
                        assert_eq!(moved, before, "{name} {bone} at {seconds}s x{scale}");
                        samples += 1;
                    }
                }
            }
        }
    }
    assert!(samples > 10_000, "{samples} samples");
}
