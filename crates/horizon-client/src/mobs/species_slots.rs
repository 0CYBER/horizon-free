//! The slots of a mob loop's snapshot by species, so a search for one species reads its few
//! entries instead of every entry near it (vanilla's `getEntitiesOfClass` reads per class).
use super::{Seen, Species};

/// Slots of the snapshot grouped by species, ascending within each.
#[derive(Default)]
pub struct SpeciesSlots {
    order: Vec<u32>,
    starts: Vec<u32>,
}

impl SpeciesSlots {
    /// Counting sort of the snapshot's slots by species.
    pub fn new(snapshot: &[Seen]) -> Self {
        let kinds = snapshot
            .iter()
            .map(|s| s.species as usize + 1)
            .max()
            .unwrap_or(0);
        let mut starts = vec![0u32; kinds + 1];
        for s in snapshot {
            starts[s.species as usize + 1] += 1;
        }
        for i in 0..kinds {
            starts[i + 1] += starts[i];
        }
        let mut next = starts.clone();
        let mut order = vec![0u32; snapshot.len()];
        for (slot, s) in snapshot.iter().enumerate() {
            let at = &mut next[s.species as usize];
            order[*at as usize] = slot as u32;
            *at += 1;
        }
        Self { order, starts }
    }
    /// The slots holding `species`, ascending.
    #[inline]
    pub fn of(&self, species: Species) -> &[u32] {
        let i = species as usize;
        match (self.starts.get(i), self.starts.get(i + 1)) {
            (Some(&a), Some(&b)) => &self.order[a as usize..b as usize],
            _ => &[],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mobs::{Mob, seen_grid::SeenGrid};
    use horizon_core::geometry::{Aabb, Vec3};
    use std::sync::Arc;

    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self, n: u64) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (self.0 >> 33) % n
        }
        fn unit(&mut self) -> f64 {
            self.next(1 << 20) as f64 / (1 << 20) as f64
        }
    }

    fn around(p: Vec3) -> Aabb {
        Aabb::new(
            Vec3::new(p.x - 0.3, p.y, p.z - 0.3),
            Vec3::new(p.x + 0.3, p.y + 1.8, p.z + 0.3),
        )
    }

    /// `nearby_species` answers as `nearby` does: the same mobs in the same order, with the grid
    /// and without it, with entries moved, killed and boxed away from their position.
    #[test]
    #[ignore = "daily"]
    fn species_search_is_the_grid_search() {
        let root = std::env::var_os("HORIZON_TEST_RESOURCES").expect("prepared resources");
        assert!(crate::mobs::world::with_resource_test_world(
            std::path::Path::new(&root),
            &[],
            true,
            true,
            |_, _| {},
            |w, types| {
                let items = w.items.expect("items");
                let kinds = [
                    "minecraft:zombie",
                    "minecraft:villager",
                    "minecraft:iron_golem",
                    "minecraft:cow",
                    "minecraft:turtle",
                    "minecraft:wolf",
                ];
                let mut rng = Lcg(99);
                for (n, spread) in [(400usize, 120.0), (1500, 60.0)] {
                    let mut snapshot: Vec<Seen> = (0..n)
                        .map(|i| {
                            let kind = Arc::new(types.types[kinds[rng.next(6) as usize]].clone());
                            let at = Vec3::new(
                                rng.unit() * spread - spread / 2.0,
                                64.0 + rng.next(4) as f64,
                                rng.unit() * spread - spread / 2.0,
                            );
                            let mut s = Mob::new(i as u32 + 1, kind, at, 0.0).seen(None, items);
                            s.alive = rng.next(8) != 0;
                            s
                        })
                        .collect();
                    let mut grid = SeenGrid::new(snapshot.iter().map(|s| s.bounds));
                    // Entries move (the grid is told), and a few are boxed away from where they
                    // stand (the box says one place, the position another).
                    for i in 0..n {
                        if rng.next(3) == 0 {
                            let to = Vec3::new(
                                rng.unit() * spread - spread / 2.0,
                                64.0,
                                rng.unit() * spread - spread / 2.0,
                            );
                            snapshot[i].position = to;
                            snapshot[i].bounds = around(to);
                            grid.update(i, snapshot[i].bounds);
                        }
                        if rng.next(40) == 0 {
                            let far = snapshot[i].position.add_vector(Vec3::new(50.0, 0.0, 0.0));
                            snapshot[i].bounds = around(far);
                            grid.update(i, snapshot[i].bounds);
                        }
                    }
                    let slots = SpeciesSlots::new(&snapshot);
                    for with_grid in [true, false] {
                        let mut view = w.clone();
                        view.others = &snapshot;
                        view.others_grid = with_grid.then_some(&grid);
                        view.species_slots = Some(&slots);
                        for _ in 0..400 {
                            let from = Vec3::new(
                                rng.unit() * spread - spread / 2.0,
                                64.0 + rng.next(4) as f64,
                                rng.unit() * spread - spread / 2.0,
                            );
                            let range = [8.0, 16.0, 35.0, 64.0][rng.next(4) as usize];
                            let wanted: Vec<Species> = match rng.next(3) {
                                0 => vec![Species::Villager],
                                1 => vec![Species::Villager, Species::IronGolem],
                                _ => vec![Species::Turtle, Species::Cow, Species::Zombie],
                            };
                            let me = rng.next(n as u64) as u32 + 1;
                            let pred = |s: &Seen| s.id != me && wanted.contains(&s.species);
                            let old: Vec<u32> =
                                view.nearby(from, range, pred).map(|s| s.id).collect();
                            let new: Vec<u32> = view
                                .nearby_species(from, range, &wanted, pred)
                                .map(|s| s.id)
                                .collect();
                            assert_eq!(
                                old, new,
                                "n {n} grid {with_grid} from {from:?} range {range}"
                            );
                        }
                    }
                }
            },
        ));
    }
}
