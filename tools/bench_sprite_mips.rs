// Compiled by bench_sprite_mips.py with the production gfx functions and loaders.
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
use std::{hint::black_box, time::Instant};

#[cfg(not(feature = "cached"))]
struct AnimatedSprites {
    animations: Vec<(SpriteAnimation, usize)>,
    size: u32,
    settings: Vec<mipmap::Settings>,
}
#[cfg(not(feature = "cached"))]
impl AnimatedSprites {
    fn new(animations: &[SpriteAnimation], size: u32, settings: &[mipmap::Settings]) -> Self {
        Self {
            animations: animations
                .iter()
                .cloned()
                .map(|a| (a, usize::MAX))
                .collect(),
            size,
            settings: settings.to_vec(),
        }
    }
}
fn update(state: &mut AnimatedSprites, tick: u64) {
    let mut sink = |layer, level, size, pixels: &[u8]| {
        black_box((layer, level, size, pixels));
    };
    #[cfg(feature = "cached")]
    {
        let levels = state.size.trailing_zeros() + 1;
        animate_sprites(state, tick, levels, &mut sink);
    }
    #[cfg(not(feature = "cached"))]
    animate_sprites(
        &mut state.animations,
        tick,
        state.size,
        state.size.trailing_zeros() + 1,
        &state.settings,
        &mut sink,
    );
}
fn retained(state: &AnimatedSprites) -> usize {
    #[cfg(feature = "cached")]
    {
        state.cache_bytes
    }
    #[cfg(not(feature = "cached"))]
    {
        let _ = state;
        0
    }
}
fn samples(name: &str, mut run: impl FnMut() -> (f64, usize)) {
    run(); // Warm code and allocator; each cold/unique run still has a new cache.
    let rows: Vec<_> = (0..9).map(|_| run()).collect();
    println!("{name} us,bytes: {rows:?}");
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let start = Instant::now();
    let mut sprites = sprites::Sprites::default();
    for name in std::fs::read_to_string(&args[2])?.lines() {
        sprites.insert(std::path::Path::new(&args[1]), name)?;
    }
    sprites.finish();
    println!(
        "resources_ms={:.3} animations={} size={}",
        start.elapsed().as_secs_f64() * 1e3,
        sprites.animations.len(),
        sprites.size
    );
    let start = Instant::now();
    let mut first = AnimatedSprites::new(&sprites.animations, sprites.size, &sprites.mips);
    update(&mut first, 0);
    let bytes = retained(&first);
    drop(first);
    println!(
        "process_first_frame_us={:.3} bytes={bytes}",
        start.elapsed().as_secs_f64() * 1e6
    );
    samples("setup+cold+drop", || {
        let start = Instant::now();
        let mut state = AnimatedSprites::new(&sprites.animations, sprites.size, &sprites.mips);
        update(&mut state, 0);
        let bytes = retained(&state);
        drop(state);
        (start.elapsed().as_secs_f64() * 1e6, bytes)
    });
    let start = Instant::now();
    let mut state = AnimatedSprites::new(&sprites.animations, sprites.size, &sprites.mips);
    for tick in 0..4096 {
        update(&mut state, tick);
    }
    println!(
        "setup+4096ticks_ms={:.3} bytes={}",
        start.elapsed().as_secs_f64() * 1e3,
        retained(&state)
    );
    #[cfg(feature = "cached")]
    println!(
        "cache_entries={} map_capacity={}",
        state.cache.len(),
        state.cache.capacity()
    );
    samples("repeated/tick", || {
        let start = Instant::now();
        for tick in 4096..5120 {
            update(&mut state, tick);
        }
        (
            start.elapsed().as_secs_f64() * 1e6 / 1024.0,
            retained(&state),
        )
    });
    samples("repeated/render60hz", || {
        let start = Instant::now();
        for frame in 0..3072 {
            update(&mut state, 4096 + frame / 3);
        }
        (
            start.elapsed().as_secs_f64() * 1e6 / 3072.0,
            retained(&state),
        )
    });
    let mut latencies: Vec<_> = (5120..6144)
        .map(|tick| {
            let start = Instant::now();
            update(&mut state, tick);
            start.elapsed().as_secs_f64() * 1e6
        })
        .collect();
    latencies.sort_unstable_by(f64::total_cmp);
    println!(
        "repeat_tick_latency_us p50={:.3} p95={:.3} max={:.3}",
        latencies[512], latencies[972], latencies[1023]
    );
    drop(state); // One live renderer cache at a time, including for RSS samples.
    // Each actual source frame is requested once, without warming its cache.
    let unique_ticks: Vec<Vec<u64>> = sprites
        .animations
        .iter()
        .map(|a| {
            let mut seen = std::collections::BTreeSet::new();
            (0..4096)
                .filter(|&tick| seen.insert(a.frame(tick)))
                .collect()
        })
        .collect();
    println!(
        "unique_source_frames={}",
        unique_ticks.iter().map(Vec::len).sum::<usize>()
    );
    samples("unique_source_frames/setup+drop (peak bytes)", || {
        let start = Instant::now();
        let mut peak = 0;
        for (animation, ticks) in sprites.animations.iter().zip(&unique_ticks) {
            let mut state =
                AnimatedSprites::new(std::slice::from_ref(animation), sprites.size, &sprites.mips);
            for &tick in ticks {
                update(&mut state, tick);
            }
            peak = peak.max(retained(&state));
        }
        (start.elapsed().as_secs_f64() * 1e6, peak)
    });
    // Distinct array layers, sharing immutable real frame inputs. Every request
    // is a first-seen key; tests admission overhead and budget saturation.
    let unique: Vec<_> = (0..8192)
        .map(|i| {
            let mut animation = sprites.animations[i % sprites.animations.len()].clone();
            animation.layer = i as u32;
            animation
        })
        .collect();
    let settings: Vec<_> = (0..unique.len())
        .map(|i| sprites.mips[sprites.animations[i % sprites.animations.len()].layer as usize])
        .collect();
    samples("unique8192/setup+first+drop", || {
        let start = Instant::now();
        let mut state = AnimatedSprites::new(&unique, sprites.size, &settings);
        update(&mut state, 0);
        let bytes = retained(&state);
        drop(state);
        (start.elapsed().as_secs_f64() * 1e6, bytes)
    });
    // Compare the complete write stream, including frames with interpolation
    // metadata, duplicate ticks, jumps, rewind and changing upload mip limits.
    let mut state = AnimatedSprites::new(&sprites.animations, sprites.size, &sprites.mips);
    let mut reference: Vec<_> = sprites
        .animations
        .iter()
        .cloned()
        .map(|a| (a, usize::MAX))
        .collect();
    for tick in (0..512).chain([511, 0, 9999, 9999, 17]) {
        let levels = (tick % (sprites.size.trailing_zeros() as u64 + 2)) as u32;
        let mut expected = Vec::new();
        reference_animate_sprites(
            &mut reference,
            tick,
            sprites.size,
            levels,
            &sprites.mips,
            |layer, level, edge, pixels| expected.push((layer, level, edge, pixels.to_vec())),
        );
        let mut actual = Vec::new();
        #[cfg(feature = "cached")]
        animate_sprites(&mut state, tick, levels, |layer, level, edge, pixels| {
            actual.push((layer, level, edge, pixels.to_vec()))
        });
        #[cfg(not(feature = "cached"))]
        animate_sprites(
            &mut state.animations,
            tick,
            state.size,
            levels,
            &state.settings,
            |layer, level, edge, pixels| actual.push((layer, level, edge, pixels.to_vec())),
        );
        assert_eq!(actual, expected, "write stream at tick {tick}");
    }
    println!("write_stream_parity=passed (517 ticks, real animations, varying mip limits)");
    Ok(())
}
