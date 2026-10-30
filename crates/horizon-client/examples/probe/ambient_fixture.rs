//! Replays `tools/test_ambient_sound_parity.py` scripts through Horizon's
//! ambient sound handlers over its `SoundEngine`:
//! `ambient_fixture ROOT INPUT OUTPUT`.
use crate::audio_common as common;
use common::{Fake, d, f, flush};
use horizon_client::ambience::{Ambience, AmbientContext, AmbientSounds, Mood, bubble_column_in};
use horizon_client::sound_engine::{Bound, SoundEngine, TickContext, Ticking};
use horizon_client::sound_events::{SoundRegistry, normalize};
use horizon_core::random::RandomSource;
use std::io::Write;

/// The oracle's scripted brightness.
fn light(mode: i32, x: i32, y: i32, z: i32) -> (u8, u8) {
    let h = x.wrapping_mul(73856093) ^ y.wrapping_mul(19349663) ^ z.wrapping_mul(83492791);
    let u = h as u32;
    match mode {
        0 => (15, 0),
        1 => (0, 0),
        2 => (0, ((u >> 4) & 15) as u8),
        _ => (
            if h & 3 == 0 { ((u >> 8) & 15) as u8 } else { 0 },
            ((u >> 12) & 15) as u8,
        ),
    }
}

fn sounds(a: &[&str]) -> AmbientSounds {
    AmbientSounds {
        r#loop: (a[1] != "-").then(|| normalize(a[1])),
        mood: (a[2] != "-").then(|| {
            let m: Vec<&str> = a[2].split(',').collect();
            Mood {
                sound: normalize(m[0]),
                tick_delay: m[1].parse().expect("delay"),
                block_search_extent: m[2].parse().expect("extent"),
                offset: d(m[3]),
            }
        }),
        additions: a[3..]
            .iter()
            .map(|s| {
                let p: Vec<&str> = s.split(',').collect();
                (normalize(p[0]), d(p[1]))
            })
            .collect(),
    }
}

pub fn main() {
    let args: Vec<String> = crate::args().collect();
    let mut engine = SoundEngine::new(SoundRegistry::load(std::path::Path::new(&args[1])));
    engine.listening = true;
    let mut backend = Fake::default();
    let mut ambience = Ambience::new(0);
    let mut current = AmbientSounds::default();
    let (mut position, mut eye, mut eye_in_water, mut spectator, mut removed, mut mode) =
        ([0.0f64; 3], 0.0f32, false, false, false, 0);
    let mut height = 1.8f32;
    let mut columns: Vec<([i32; 3], bool)> = Vec::new();
    let input = std::fs::read_to_string(&args[2]).expect("input");
    let mut out = std::io::BufWriter::new(std::fs::File::create(&args[3]).expect("output"));
    for line in input.lines() {
        writeln!(out, "{line}").expect("write");
        let a: Vec<&str> = line.split(' ').collect();
        match a[0] {
            "start" => {
                engine.stop_all(&mut backend);
                ambience.clear(&mut Bound {
                    engine: &mut engine,
                    backend: &mut backend,
                });
                ambience = Ambience::new(a[1].parse().expect("seed"));
                current = AmbientSounds::default();
                (position, eye, eye_in_water, spectator, removed, mode) =
                    ([0.0; 3], 0.0, false, false, false, 0);
                height = 1.8;
                columns.clear();
            }
            "sounds" => current = sounds(&a),
            "player" => {
                position = [d(a[1]), d(a[2]), d(a[3])];
                eye = f(a[4]);
                eye_in_water = a[5] == "1";
                spectator = a[6] == "1";
                removed = a[7] == "1";
                mode = a[8].parse().expect("mode");
                height = f(a[9]);
                columns = a[10..]
                    .iter()
                    .map(|c| {
                        let p: Vec<&str> = c.split(',').collect();
                        let n = |i: usize| p[i].parse::<i32>().expect("column");
                        ([n(0), n(1), n(2)], p[3] == "1")
                    })
                    .collect();
            }
            "volume" => {
                let source: usize = a[1].parse().expect("source");
                engine.source_volumes[source] = f(a[2]);
                engine.refresh_category_volume(source, &mut backend);
            }
            "tick" => {
                let light = |b: [i32; 3]| light(mode, b[0], b[1], b[2]);
                let ctx = AmbientContext {
                    sounds: &current,
                    position,
                    eye_y: position[1] + f64::from(eye),
                    underwater: eye_in_water,
                    spectator,
                    light: &light,
                    bubble_column: bubble_column_in(
                        [
                            position[0] - f64::from(0.3f32),
                            position[1],
                            position[2] - f64::from(0.3f32),
                            position[0] + f64::from(0.3f32),
                            position[1] + f64::from(height),
                            position[2] + f64::from(0.3f32),
                        ],
                        |b| columns.iter().find(|c| c.0 == b).map(|c| c.1),
                    ),
                };
                ambience.tick(
                    &ctx,
                    &mut Bound {
                        engine: &mut engine,
                        backend: &mut backend,
                    },
                );
                let tick = TickContext {
                    player_underwater: ambience.was_underwater(),
                    player_removed: removed,
                    ..TickContext::default()
                };
                engine.tick(false, &tick, &mut backend);
            }
            _ => panic!("{line}"),
        }
        writeln!(
            out,
            "  rand={} mood={:08x} underwater={} delay={}",
            ambience.random.clone().next_long(),
            ambience.moodiness.to_bits(),
            ambience.was_underwater(),
            ambience.underwater_delay()
        )
        .expect("write");
        for (event, id) in ambience.loops() {
            let i = engine.instance(*id).expect("loop");
            let Ticking::BiomeLoop { fade, direction } = i.ticking else {
                panic!("loop kind");
            };
            writeln!(
                out,
                "  loop {event} fade={fade} direction={direction} stopped={} active={}",
                i.stopped,
                engine.is_active(*id)
            )
            .expect("write");
        }
        let mut playing: Vec<String> = engine
            .active_instances()
            .into_iter()
            .map(|id| {
                let i = engine.instance(id).expect("instance");
                format!(
                    "  playing {} volume={:08x} looping={} relative={} at={:016x},{:016x},{:016x} stopped={}",
                    i.identifier,
                    i.volume.to_bits(),
                    i.looping,
                    i.relative,
                    i.position[0].to_bits(),
                    i.position[1].to_bits(),
                    i.position[2].to_bits(),
                    i.tickable() && i.stopped
                )
            })
            .collect();
        playing.sort();
        for p in playing {
            writeln!(out, "{p}").expect("write");
        }
        flush(&mut engine, &mut out);
    }
}
