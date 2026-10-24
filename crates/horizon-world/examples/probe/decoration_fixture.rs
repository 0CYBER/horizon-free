use horizon_core::{position::ChunkPos, random::RandomSource};
use horizon_world::{
    heightmap::HeightmapType,
    worldgen::{
        anchor::WorldGenerationContext,
        decoration::{DecorationBiome, DecorationWorld, FeatureDecorationPlan},
        feature_sort::FeatureIdentity,
        placement::{PlacementContext, PlacementModifier, PlacementQueryError},
    },
};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
fn n(v: &Value) -> i32 {
    v.as_i64().unwrap() as i32
}
struct World<'a, W> {
    f: &'a Value,
    out: &'a mut W,
    biomes: &'a [Vec<Vec<FeatureIdentity>>],
    calls: i32,
    queries: i32,
    notifications: i32,
    chunks_read: i32,
}
impl<W: Write> DecorationWorld for World<'_, W> {
    type Error = ();
    fn min_section_y(&mut self) -> Result<i32, ()> {
        Ok(n(&self.f["min"]) >> 4)
    }
    fn chunk_biomes(&mut self, chunk: ChunkPos) -> Result<Vec<DecorationBiome>, ()> {
        writeln!(self.out, "chunk {} {}", chunk.x, chunk.z).map_err(|_| ())?;
        let call = self.chunks_read;
        self.chunks_read += 1;
        if call == n(&self.f["chunk_fail"]) {
            return Err(());
        }
        let dx = chunk.x.wrapping_sub(n(&self.f["cx"]));
        let dz = chunk.z.wrapping_sub(n(&self.f["cz"]));
        let values = self.f["palettes"][((dz + 1) * 3 + dx + 1) as usize]
            .as_array()
            .unwrap();
        // The native fixture fills every quart cell from this repeating sequence.
        // Only cells actually present in the chunk count as resident biomes.
        Ok((0..n(&self.f["height"]) * 4)
            .map(|i| DecorationBiome(n(&values[i as usize % values.len()]) as u32))
            .collect())
    }
    fn set_currently_generating(&mut self, feature: Option<FeatureIdentity>) -> Result<(), ()> {
        match feature {
            Some(id) => writeln!(
                self.out,
                "generating ResourceKey[minecraft:worldgen/placed_feature / horizon:f{}]",
                id.0
            ),
            None => writeln!(self.out, "generating none"),
        }
        .map_err(|_| ())?;
        let call = self.notifications;
        self.notifications += 1;
        if call == n(&self.f["notify_fail"]) {
            Err(())
        } else {
            Ok(())
        }
    }
}
impl<W: Write> PlacementContext for World<'_, W> {
    fn generation(&self) -> WorldGenerationContext {
        WorldGenerationContext::new(-64, 384, 63)
    }
    fn min_y(&mut self) -> Result<i32, PlacementQueryError> {
        Ok(n(&self.f["min"]))
    }
    fn height(&mut self, _: HeightmapType, _: i32, _: i32) -> Result<i32, PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }
    fn biome_allows_feature(
        &mut self,
        p: [i32; 3],
        feature: FeatureIdentity,
    ) -> Result<bool, PlacementQueryError> {
        let index = p[0]
            .wrapping_mul(31)
            .wrapping_add(p[1].wrapping_mul(7))
            .wrapping_add(p[2].wrapping_mul(17))
            .wrapping_add(n(&self.f["biome_offset"]))
            .rem_euclid(self.biomes.len() as i32 + 1) as usize;
        writeln!(self.out, "biome {} {} {} {}", p[0], p[1], p[2], index)
            .map_err(|e| PlacementQueryError::Failed(e.to_string()))?;
        let call = self.queries;
        self.queries += 1;
        if call == n(&self.f["query_fail"]) {
            return Err(PlacementQueryError::Unavailable);
        }
        Ok(self
            .biomes
            .get(index)
            .is_some_and(|steps| steps.iter().flatten().any(|id| *id == feature)))
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).expect("private fixture directory");
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let biomes = f["biomes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| {
                b.as_array()
                    .unwrap()
                    .iter()
                    .map(|s| {
                        s.as_array()
                            .unwrap()
                            .iter()
                            .map(|id| FeatureIdentity(n(id) as u32))
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let plan = FeatureDecorationPlan::new(
            biomes
                .iter()
                .enumerate()
                .map(|(i, b)| (DecorationBiome(i as u32), b.clone()))
                .collect(),
        );
        let Ok(plan) = plan else {
            writeln!(out, "plan error")?;
            continue;
        };
        writeln!(out, "plan ok")?;
        let placements = f["placements"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| {
                p.as_array()
                    .unwrap()
                    .iter()
                    .map(|m| PlacementModifier::from_json(&m.to_string()))
                    .collect::<Result<Vec<_>, _>>()
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut world = World {
            f: &f,
            out: &mut out,
            biomes: &biomes,
            calls: 0,
            queries: 0,
            notifications: 0,
            chunks_read: 0,
        };
        for _ in 0..2 {
            world.calls = 0;
            world.queries = 0;
            world.notifications = 0;
            world.chunks_read = 0;
            let result = plan.decorate_features(
                &mut world,
                ChunkPos::new(n(&f["cx"]), n(&f["cz"])),
                f["seed"].as_i64().unwrap(),
                |w, placer, rng, id, origin| {
                    placer
                        .place_with_biome_check(
                            &placements[id.0 as usize],
                            w,
                            rng,
                            origin,
                            id,
                            |w, r, p| {
                                writeln!(w.out, "feature {} {} {} {}", id.0, p[0], p[1], p[2])
                                    .map_err(|_| ())?;
                                writeln!(
                                    w.out,
                                    "draw {} {} {:x} {:x}",
                                    r.next_long(),
                                    r.next_int_bound(17).unwrap(),
                                    r.next_float().to_bits(),
                                    r.next_double().to_bits()
                                )
                                .map_err(|_| ())?;
                                let call = w.calls;
                                w.calls += 1;
                                if call == n(&w.f["feature_fail"]) {
                                    return Err(());
                                }
                                Ok(n(&w.f["return_mode"]) == 0
                                    || (n(&w.f["return_mode"]) == 2 && id.0 % 2 == 0))
                            },
                        )
                        .map_err(|_| ())
                },
            );
            writeln!(
                world.out,
                "decorate {}",
                if result.is_ok() { "ok" } else { "error" }
            )?;
        }
    }
    Ok(())
}
