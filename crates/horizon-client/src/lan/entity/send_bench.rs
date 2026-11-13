//! Opt-in measurements of this PR's send strategies, with identical inputs and packet checks.
//! See docs/performance/tracker-send.md. Normal tests and `--ignored` alone do not measure.

use super::*;
use std::time::{Duration, Instant};

#[cfg(feature = "tracker-send-bench")]
#[global_allocator]
static ALLOCATOR: horizon_threads::GameAllocator = horizon_threads::GAME_ALLOCATOR;

struct Scene {
    level: Entities,
    entities: Vec<EntityState>,
    trackers: Vec<Tracker>,
    out: Vec<Vec<Vec<u8>>>,
    chunks: crate::fast_hash::FxHashSet<(i32, i32)>,
    active: Vec<usize>,
}

impl Scene {
    fn new(viewers: usize, entities: usize, parallel: bool) -> Self {
        let level = Entities {
            send_apart_always: Some(parallel),
            ..Entities::default()
        };
        Self {
            level,
            entities: (0..entities)
                .map(|i| EntityState {
                    id: i as i32 + 1,
                    uuid: i as u128 + 1,
                    kind: if i < viewers {
                        "minecraft:player"
                    } else {
                        "minecraft:zombie"
                    }
                    .into(),
                    position: [(i % 8) as f64, 64.0, ((i / 8) % 8) as f64],
                    on_ground: true,
                    ..EntityState::default()
                })
                .collect(),
            trackers: (0..viewers).map(|_| Tracker::default()).collect(),
            out: vec![Vec::new(); viewers],
            chunks: (-2..=2)
                .flat_map(|x| (-2..=2).map(move |z| (x, z)))
                .collect(),
            active: (0..entities).collect(),
        }
    }

    fn step(&mut self, tick: usize, send_only: bool) -> Duration {
        for (i, e) in self.entities.iter_mut().enumerate() {
            e.position[0] = (i % 8) as f64 + (tick % 64) as f64 / 32.0;
            e.yaw = (tick % 120) as f32 * 3.0;
            e.head_yaw = e.yaw;
        }
        // Full-tick samples include a receiver pairing again, as on a respawn.
        if !send_only && tick.is_multiple_of(8) {
            let index = tick % self.trackers.len();
            self.trackers[index] = Tracker::default();
        }
        for out in &mut self.out {
            out.clear();
        }
        let refs: Vec<_> = self.entities.iter().collect();
        let mut viewers: Vec<_> = self
            .trackers
            .iter_mut()
            .zip(&mut self.out)
            .enumerate()
            .map(|(i, (tracker, out))| Viewer {
                entity_id: i as i32 + 1,
                spectator: false,
                permission: 0,
                moved: true,
                position: self.entities[i].position,
                view_distance: 10,
                chunks: &self.chunks,
                tracker,
                out,
                gone: None,
            })
            .collect();
        let riding = Riding::default();
        let started = Instant::now();
        if !send_only {
            self.level.tick(&refs, &riding, &mut viewers, |_| Some(5));
        } else if self.level.send_apart_always == Some(true) {
            self.level
                .send_on_threads(&refs, &riding, &mut viewers, &self.active);
        } else {
            self.level
                .send_in_order(&refs, &riding, &mut viewers, &self.active);
        }
        started.elapsed()
    }

    fn events(&mut self) -> Vec<Vec<(bool, i32)>> {
        self.trackers
            .iter_mut()
            .map(|t| {
                t.take_events()
                    .into_iter()
                    .map(|e| (e.start, e.entity))
                    .collect()
            })
            .collect()
    }
}

#[test]
#[ignore = "performance measurement; requires HORIZON_SEND_BENCH=1 on an idle machine"]
fn tracker_send_benchmark() {
    if std::env::var("HORIZON_SEND_BENCH").as_deref() != Ok("1") {
        eprintln!("Not measuring: explicitly set HORIZON_SEND_BENCH=1 on an idle machine.");
        return;
    }
    assert!(
        !std::hint::black_box(cfg!(debug_assertions)),
        "build with --profile fast"
    );
    assert!(
        std::hint::black_box(cfg!(feature = "tracker-send-bench")),
        "build with --features tracker-send-bench for the game's allocator"
    );
    assert!(
        !std::env::var("HORIZON_SERIAL_TICK").is_ok_and(|v| v.eq_ignore_ascii_case("all")),
        "HORIZON_SERIAL_TICK=all disables the parallel path being measured"
    );
    let cases = [
        (15, 63),
        (15, 64),
        (16, 63),
        (16, 64),
        (17, 65),
        (32, 128),
        (120, 120),
        (400, 400),
        (600, 600),
    ];
    println!(
        "SEND_BENCH {}",
        serde_json::json!({
            "workers": horizon_threads::worker_count(),
        "allocator": "horizon_threads::GameAllocator",
            "threshold_viewers": SEND_APART_VIEWERS,
            "threshold_entities": SEND_APART_ENTITIES,
        })
    );
    for (viewers, entities) in cases {
        for send_only in [true, false] {
            for round in 0..7 {
                let mut scenes = [
                    Scene::new(viewers, entities, false),
                    Scene::new(viewers, entities, true),
                ];
                // Establish TRACKS flags and warm both paths before timing steady-state sends.
                for tick in 1..=10 {
                    for scene in &mut scenes {
                        scene.step(tick, false);
                    }
                    assert_eq!(scenes[0].out, scenes[1].out);
                    let [serial, parallel] = &mut scenes;
                    assert_eq!(serial.events(), parallel.events());
                }
                let mut samples: [Vec<u128>; 2] = std::array::from_fn(|_| Vec::new());
                let mut packets = 0;
                for tick in 11..=30 {
                    // Alternate which strategy runs first, including between rounds.
                    let first = (tick + round) % 2;
                    for side in [first, 1 - first] {
                        samples[side].push(scenes[side].step(tick, send_only).as_nanos());
                    }
                    assert_eq!(scenes[0].out, scenes[1].out);
                    packets += scenes[0].out.iter().map(Vec::len).sum::<usize>();
                    let [serial, parallel] = &mut scenes;
                    assert_eq!(serial.events(), parallel.events());
                }
                assert!(packets > 0, "the scene must send packets");
                println!(
                    "SEND_BENCH {}",
                    serde_json::json!({
                        "viewers": viewers, "entities": entities, "round": round,
                        "phase": if send_only { "send" } else { "tick_with_pairing" },
                        "serial_ns": samples[0], "parallel_ns": samples[1], "packets": packets,
                    })
                );
            }
        }
    }
}
