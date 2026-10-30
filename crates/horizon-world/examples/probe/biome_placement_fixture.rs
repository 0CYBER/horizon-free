use horizon_core::random::{LegacyRandom, RandomSource, WorldgenRandom, XoroshiroRandom};
use horizon_world::{
    heightmap::HeightmapType,
    worldgen::{
        anchor::WorldGenerationContext,
        feature_sort::FeatureIdentity,
        placement::{FeaturePlacer, PlacementContext, PlacementModifier, PlacementQueryError},
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
    out: &'a mut W,
    f: &'a Value,
    queries: i32,
    calls: i32,
}
impl<W: Write> PlacementContext for World<'_, W> {
    fn top_feature(&self) -> Option<FeatureIdentity> {
        self.f["top"]
            .as_bool()
            .unwrap()
            .then_some(FeatureIdentity(42))
    }
    fn generation(&self) -> WorldGenerationContext {
        WorldGenerationContext::new(-64, 384, 63)
    }
    fn min_y(&mut self) -> Result<i32, PlacementQueryError> {
        Ok(-64)
    }
    fn height(&mut self, _: HeightmapType, _: i32, _: i32) -> Result<i32, PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }
    fn biome_allows_feature(
        &mut self,
        p: [i32; 3],
        feature: FeatureIdentity,
    ) -> Result<bool, PlacementQueryError> {
        assert_eq!(feature, FeatureIdentity(42));
        let error = |e: io::Error| PlacementQueryError::Failed(e.to_string());
        writeln!(self.out, "biome {} {} {}", p[0], p[1], p[2]).map_err(error)?;
        let call = self.queries;
        self.queries += 1;
        if call == n(&self.f["query_fail"]) {
            return Err(PlacementQueryError::Unavailable);
        }
        let index = p[0]
            .wrapping_mul(31)
            .wrapping_add(p[1].wrapping_mul(7))
            .wrapping_add(p[2].wrapping_mul(17))
            .rem_euclid(2) as usize;
        writeln!(self.out, "selected {index}").map_err(error)?;
        writeln!(self.out, "settings {index}").map_err(error)?;
        let call = self.queries;
        self.queries += 1;
        if call == n(&self.f["query_fail"]) {
            return Err(PlacementQueryError::Unavailable);
        }
        Ok(matches!(n(&self.f["membership"][index]), 1 | 2))
    }
}
fn feature<W: Write>(
    world: &mut World<'_, W>,
    random: &mut impl RandomSource,
    p: [i32; 3],
) -> Result<bool, ()> {
    writeln!(world.out, "feature {} {} {}", p[0], p[1], p[2]).map_err(|_| ())?;
    if world.f["draw"].as_bool().unwrap() {
        writeln!(world.out, "draw {}", random.next_long()).map_err(|_| ())?;
    }
    let call = world.calls;
    world.calls += 1;
    if call == n(&world.f["feature_fail"]) {
        return Err(());
    }
    if world.f["nested"].as_bool().unwrap() {
        return FeaturePlacer::new()
            .place_with_context(&[PlacementModifier::Biome], world, random, p, feature)
            .map_err(|_| ());
    }
    Ok(n(&world.f["return_mode"]) == 0
        || (n(&world.f["return_mode"]) == 2
            && p[0].wrapping_add(p[1]).wrapping_add(p[2]).rem_euclid(2) == 0))
}
fn fixture(
    f: &Value,
    mut random: impl RandomSource,
    out: &mut impl Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let modifiers = f["placement"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| PlacementModifier::from_json(&v.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    let origin = std::array::from_fn(|i| n(&f["origin"][i]));
    let mut world = World {
        out,
        f,
        queries: 0,
        calls: 0,
    };
    let mut placer = FeaturePlacer::new();
    for _ in 0..3 {
        world.queries = 0;
        world.calls = 0;
        let result = if f["direct"].as_bool().unwrap() {
            // Direct modifier callbacks share their query world through a local
            // emitted-position list; this corpus uses only the unit biome filter.
            let mut positions = Vec::new();
            let result = modifiers[0].modify_in(&mut world, &mut random, origin, |_, p| {
                positions.push(p);
                Ok::<_, ()>(())
            });
            result.map_err(|_| ()).and_then(|()| {
                for p in positions {
                    feature(&mut world, &mut random, p)?;
                }
                Ok(true)
            })
        } else if f["top"].as_bool().unwrap() {
            placer
                .place_with_biome_check(
                    &modifiers,
                    &mut world,
                    &mut random,
                    origin,
                    FeatureIdentity(42),
                    feature,
                )
                .map_err(|_| ())
        } else {
            placer
                .place_with_context(&modifiers, &mut world, &mut random, origin, feature)
                .map_err(|_| ())
        };
        match result {
            Ok(value) => writeln!(world.out, "place {value}")?,
            Err(()) => writeln!(world.out, "place error")?,
        }
        writeln!(world.out, "next {}", random.next_long())?;
    }
    writeln!(world.out, "tail {}", random.next_long())?;
    Ok(())
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).expect("private fixture directory");
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let seed = f["seed"].as_i64().unwrap();
        match (
            f["legacy"].as_bool().unwrap(),
            f["worldgen"].as_bool().unwrap(),
        ) {
            (true, false) => fixture(&f, LegacyRandom::new(seed), &mut out)?,
            (true, true) => fixture(&f, WorldgenRandom::new(LegacyRandom::new(seed)), &mut out)?,
            (false, false) => fixture(&f, XoroshiroRandom::new(seed), &mut out)?,
            (false, true) => fixture(
                &f,
                WorldgenRandom::new(XoroshiroRandom::new(seed)),
                &mut out,
            )?,
        }
    }
    Ok(())
}
