//! The sweep's cell test (`cells`) gives every client the bytes and tracking events of the
//! sweep that looks at each entity.
use super::*;
use std::sync::atomic::Ordering::Relaxed;

fn entity(id: i32, kind: &str, at: [f64; 2]) -> EntityState {
    EntityState {
        id,
        uuid: id as u128,
        kind: kind.into(),
        position: [at[0], 64.0, at[1]],
        yaw: 0.0,
        pitch: 0.0,
        head_yaw: 0.0,
        on_ground: true,
        velocity: [0.0; 3],
        acceleration_power: None,
        minecart_steps: None,
        data: 0,
        meta: Vec::new(),
        forced_meta: [0; 4],
        defaults: Vec::new(),
        equipment: Vec::new(),
        attributes: Vec::new(),
        precise: false,
        vertical_collision: false,
        horizontal_collision: false,
        needs_sync: false,
        sync_position: false,
        stepped: false,
        idle: false,
        bare: false,
        leash: None,
        spectator: false,
        watching: false,
        hidden: None,
    }
}

/// Crowds of players standing close to herds that mostly stand still, with entities crossing
/// sections and cells, appearing, leaving, turning spectator or hidden, and clients that move,
/// change their view distance, lose chunks and forget what they track: the same bytes and events
/// for each client, tick by tick, with the cells met whole or one entity at a time, and some
/// cells met whole (so the comparison is not of the full sweep with itself).
///
/// Not exercised: clients viewing a level from another (`sources`, never skipped), riders, a
/// client's entity changing id, the send pass on several threads (the cells only decide the
/// flags it reads).
#[test]
fn cells_met_whole_match_every_entity_looked_at() {
    let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
    let mut next = move |n: u64| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed % n
    };
    let kinds = ["minecraft:cow", "minecraft:sheep", "minecraft:item"];
    let full: crate::fast_hash::FxHashSet<(i32, i32)> =
        (-6..6).flat_map(|x| (-6..6).map(move |z| (x, z))).collect();
    let skipped_before = cells::SKIPPED.load(Relaxed);
    let mut compared = 0;
    for level in 0..16 {
        let mut both = [Entities::default(), Entities::default()];
        both[1].cells.off = true;
        let players = 2 + next(12) as i32;
        let mut trackers: [Vec<Tracker>; 2] =
            std::array::from_fn(|_| (0..players).map(|_| Tracker::default()).collect());
        let mut outs: [Vec<Vec<crate::lan::Body>>; 2] =
            std::array::from_fn(|_| vec![Vec::new(); players as usize]);
        let mut held: Vec<crate::fast_hash::FxHashSet<(i32, i32)>> =
            (0..players).map(|_| full.clone()).collect();
        let at = |next: &mut dyn FnMut(u64) -> u64| {
            [
                (next(1000) as f64 / 1000.0 - 0.5) * 40.0,
                (next(1000) as f64 / 1000.0 - 0.5) * 40.0,
            ]
        };
        let mut all: Vec<EntityState> = (0..players + 40 + next(150) as i32)
            .map(|i| {
                let kind = if i < players {
                    "minecraft:player"
                } else {
                    kinds[next(3) as usize]
                };
                entity(1 + i, kind, at(&mut next))
            })
            .collect();
        let mut next_id = all.len() as i32 + 1;
        let mut view = vec![6; players as usize];
        for tick in 0..40 {
            if next(5) == 0 {
                all.push(entity(next_id, kinds[next(3) as usize], at(&mut next)));
                next_id += 1;
            }
            if next(6) == 0 && all.len() > players as usize + 1 {
                let gone = players as usize + next((all.len() - players as usize) as u64) as usize;
                all.remove(gone);
            }
            for e in &mut all {
                let player = e.kind == "minecraft:player";
                // Herds stand, players walk, and now and then one is moved across a cell.
                if (player && next(2) == 0) || next(40) == 0 {
                    let far = if next(8) == 0 { 40.0 } else { 1.5 };
                    e.position[0] += (next(1000) as f64 / 1000.0 - 0.5) * far;
                    e.position[2] += (next(1000) as f64 / 1000.0 - 0.5) * 1.5;
                }
                if next(60) == 0 {
                    e.spectator = player && !e.spectator;
                }
                if next(80) == 0 {
                    e.hidden = if e.hidden.is_some() {
                        None
                    } else {
                        Some(next(3) as u8)
                    };
                }
                if next(15) == 0 {
                    e.meta = vec![(0, serializer::BYTE, Value::Byte(next(3) as i8))];
                }
            }
            for k in 0..players as usize {
                if next(30) == 0 {
                    view[k] = 1 + next(8) as i32;
                }
                if next(25) == 0 {
                    let cell = (next(12) as i32 - 6, next(12) as i32 - 6);
                    if !held[k].remove(&cell) {
                        held[k].insert(cell);
                    }
                }
            }
            let refs: Vec<&EntityState> = all.iter().collect();
            let moved: Vec<bool> = (0..players).map(|_| tick == 0 || next(5) != 0).collect();
            let forget: Vec<Option<i32>> = (0..players)
                .map(|_| (next(40) == 0).then(|| 1 + next(60) as i32))
                .collect();
            for side in 0..2 {
                for out in &mut outs[side] {
                    out.clear();
                }
                for (tracker, forget) in trackers[side].iter_mut().zip(&forget) {
                    if let Some(id) = *forget {
                        tracker.forget(id);
                    }
                }
                let mut viewers: Vec<Viewer> = trackers[side]
                    .iter_mut()
                    .zip(outs[side].iter_mut())
                    .enumerate()
                    .map(|(k, (tracker, out))| Viewer {
                        entity_id: 1 + k as i32,
                        horizon: true,
                        sources: None,
                        spectator: all[k].spectator,
                        permission: (k % 3) as u8,
                        moved: moved[k],
                        position: all[k].position,
                        view_distance: view[k],
                        chunks: &held[k],
                        tracker,
                        out,
                        gone: None,
                    })
                    .collect();
                both[side].tick(&refs, &Riding::default(), &mut viewers, |_| Some(5));
            }
            let events = |trackers: &mut Vec<Tracker>| -> Vec<Vec<(bool, i32)>> {
                trackers
                    .iter_mut()
                    .map(|t| {
                        t.take_events()
                            .iter()
                            .map(|e| (e.start, e.entity))
                            .collect()
                    })
                    .collect()
            };
            let [first, second] = &mut trackers;
            assert_eq!(events(first), events(second), "level {level}, tick {tick}");
            assert_eq!(outs[0], outs[1], "level {level}, tick {tick}");
            compared += outs[0].iter().map(Vec::len).sum::<usize>();
        }
    }
    assert!(
        compared > 5_000,
        "the clients were sent their entities ({compared})"
    );
    let skipped = cells::SKIPPED.load(Relaxed) - skipped_before;
    assert!(skipped > 500, "the sweep met cells whole ({skipped})");
}
