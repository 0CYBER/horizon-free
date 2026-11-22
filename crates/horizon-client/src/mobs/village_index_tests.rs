use super::*;
use crate::mobs::{Mob, villager_brain::poi_at};
use horizon_core::geometry::Vec3;
use std::sync::Arc;

/// `village_sections` and `sections_to_village` as they were: every villager of the snapshot read
/// again at each question, the sections in a list scanned for the least distance.
fn reference(w: &MobWorld, own: &[Section], at: Section) -> i32 {
    let mut out: Vec<Section> = Vec::new();
    let mut add = |p: [i32; 3]| {
        let s = [p[0] >> 4, p[1] >> 4, p[2] >> 4];
        if !out.contains(&s) {
            out.push(s);
        }
    };
    for s in own {
        add([s[0] << 4, s[1] << 4, s[2] << 4]);
    }
    for o in w.others {
        if o.alive && o.species == Species::Villager {
            for p in [o.job, o.bed, o.potential_job, o.meeting]
                .into_iter()
                .flatten()
            {
                if poi_at(w, p[0], p[1], p[2]).is_some() {
                    add(p);
                }
            }
        }
    }
    out.iter()
        .map(|v| {
            (v[0] - at[0])
                .abs()
                .max((v[1] - at[1]).abs())
                .max((v[2] - at[2]).abs())
        })
        .min()
        .unwrap_or(7)
        .min(7)
}

struct Lcg(u64);
impl Lcg {
    fn next(&mut self, n: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) % n
    }
}

#[test]
#[ignore = "daily"]
fn index_answers_as_the_scan_of_every_villager_did() {
    let root = std::env::var_os("HORIZON_TEST_RESOURCES").expect("prepared resources");
    let pois = [
        "minecraft:bell",
        "minecraft:lectern",
        "minecraft:composter",
        "minecraft:barrel",
    ];
    // Candidate sites on a 24 x 24 grid, 12 blocks apart over 22 sections: every third is no POI.
    let site = |i: u64| {
        let (gx, gz) = ((i % 24) as i32, (i / 24 % 24) as i32);
        [gx * 12 - 140, 64 + (i / 576) as i32 % 3, gz * 12 - 140]
    };
    assert!(crate::mobs::world::with_resource_test_world(
        std::path::Path::new(&root),
        &[],
        true,
        true,
        |world, registry| {
            for i in 0..576 * 3 {
                let p = site(i);
                let chunk = (p[0] >> 4, p[2] >> 4);
                if !world.chunks.contains_key(&chunk) {
                    world.chunks.insert(
                        chunk,
                        Arc::new(crate::chunk::Chunk::empty(chunk.0, chunk.1)),
                    );
                }
                if i % 3 != 0 {
                    let id = pois[(i % 4) as usize];
                    world
                        .set(p[0], p[1], p[2], registry.default_state(id).unwrap())
                        .unwrap();
                }
            }
        },
        |w, types| {
            let items = w.items.expect("items");
            let kind = Arc::new(types.types["minecraft:villager"].clone());
            let mut rng = Lcg(7);
            let mut snapshot: Vec<Seen> = (0..80)
                .map(|i| {
                    let m = Mob::new(i + 1, Arc::clone(&kind), Vec3::new(0.0, 64.0, 0.0), 0.0);
                    m.seen(None, items)
                })
                .collect();
            let pick = |rng: &mut Lcg| (rng.next(3) != 0).then(|| site(rng.next(576 * 3)));
            for s in &mut snapshot {
                (s.job, s.bed, s.potential_job, s.meeting) = (
                    pick(&mut rng),
                    pick(&mut rng),
                    pick(&mut rng),
                    pick(&mut rng),
                );
            }
            let slots = |snapshot: &[Seen]| -> Vec<u32> {
                (0..snapshot.len() as u32)
                    .filter(|&i| snapshot[i as usize].species == Species::Villager)
                    .collect()
            };
            let mut index = VillageIndex::default();
            let mut checked = 0;
            for tick in 0..600u64 {
                // A pass boundary: the world may have changed, the loop starts the index again.
                if tick % 150 == 149 {
                    index = VillageIndex::default();
                }
                // The turns of a few mobs end: their entries are written, each one marked.
                for _ in 0..rng.next(6) {
                    let slot = rng.next(80) as usize;
                    let mut now = snapshot[slot].clone();
                    match rng.next(5) {
                        0 => now.alive = false,
                        1 => now.alive = true,
                        2 => {
                            now.species = if now.species == Species::Villager {
                                Species::Cow
                            } else {
                                Species::Villager
                            }
                        }
                        _ => {
                            (now.job, now.bed, now.potential_job, now.meeting) = (
                                pick(&mut rng),
                                pick(&mut rng),
                                pick(&mut rng),
                                pick(&mut rng),
                            )
                        }
                    }
                    index.mark(slot, &snapshot[slot], &now);
                    snapshot[slot] = now;
                }
                let villagers = slots(&snapshot);
                let mut view = w.clone();
                view.others = &snapshot;
                view.others_villagers = Some(&villagers);
                view.village = Some(&index);
                let mut bare = view.clone();
                bare.village = None;
                // Every mob of the tick asks, with its own sites.
                for _ in 0..8 {
                    let own: Vec<Section> = (0..rng.next(4))
                        .map(|_| {
                            let p = site(rng.next(576 * 3));
                            [p[0] >> 4, p[1] >> 4, p[2] >> 4]
                        })
                        .collect();
                    let villages = Villages::new(&view, &own);
                    let alone = Villages::new(&bare, &own);
                    for _ in 0..12 {
                        let at = [
                            rng.next(26) as i32 - 13,
                            3 + rng.next(3) as i32,
                            rng.next(26) as i32 - 13,
                        ];
                        let want = reference(&view, &own, at);
                        assert_eq!(villages.distance(at), want, "tick {tick} at {at:?}");
                        assert_eq!(alone.distance(at), want, "tick {tick} at {at:?} (no index)");
                        checked += 1;
                    }
                }
            }
            assert!(checked > 20_000);
        },
    ));
}
