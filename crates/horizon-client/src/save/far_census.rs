use super::*;
use crate::lod::Column;
use crate::streaming::ChunkGenerator;

#[derive(Default, Debug)]
struct Census {
    tiles: usize,
    floating: usize,
    cells: usize,
    kinds: std::collections::BTreeMap<&'static str, usize>,
    water_over_air: usize,
}

fn kind(name: &str) -> &'static str {
    if name.ends_with("_leaves") {
        "leaves"
    } else if name.ends_with("_log") || name.ends_with("_wood") || name.contains("stem") {
        "log"
    } else if name == "minecraft:water" || name == "minecraft:lava" {
        "fluid"
    } else if name.contains("snow") || name.contains("ice") {
        "snow/ice"
    } else {
        "ground"
    }
}

/// `columns`: `side` x `side`, the outer ring a halo.
fn count(
    columns: &[Column],
    side: usize,
    name: &dyn Fn(horizon_world::registry::BlockStateId) -> String,
    out: &mut Census,
    origin: (i32, i32, i32),
) {
    // Runs of touching spans, per column: (min, max, top state).
    let runs: Vec<Vec<(i16, i16, horizon_world::registry::BlockStateId)>> = columns
        .iter()
        .map(|column| {
            let mut runs: Vec<(i16, i16, _)> = Vec::new();
            for span in column.spans.iter().filter(|s| s.min < s.max) {
                match runs.last_mut() {
                    Some(run) if run.1 == span.min => {
                        run.1 = span.max;
                        run.2 = span.state;
                    }
                    _ => runs.push((span.min, span.max, span.state)),
                }
            }
            runs
        })
        .collect();
    let mut first = Vec::with_capacity(runs.len());
    let mut total = 0;
    for column in &runs {
        first.push(total);
        total += column.len();
    }
    let mut parent: Vec<usize> = (0..total).collect();
    fn find(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    let union = |parent: &mut Vec<usize>, a: usize, b: usize| {
        let (a, b) = (find(parent, a), find(parent, b));
        if a != b {
            parent[a] = b;
        }
    };
    for z in 0..side {
        for x in 0..side {
            let i = z * side + x;
            for j in [
                (x + 1 < side).then(|| i + 1),
                (z + 1 < side).then(|| i + side),
            ]
            .into_iter()
            .flatten()
            {
                for (a, &(lo, hi, _)) in runs[i].iter().enumerate() {
                    for (b, &(other_lo, other_hi, _)) in runs[j].iter().enumerate() {
                        // Cells meeting at an edge (an acacia's
                        // diagonal branch) look joined too.
                        if lo <= other_hi && other_lo <= hi {
                            union(&mut parent, first[i] + a, first[j] + b);
                        }
                    }
                }
            }
        }
    }
    // Per component: grounded, touches the halo, cells, top state.
    let mut grounded = vec![false; total];
    let mut halo = vec![false; total];
    let mut cells = vec![0usize; total];
    let mut top = vec![None; total];
    let mut at = vec![(0, 0); total];
    for z in 0..side {
        for x in 0..side {
            let i = z * side + x;
            let ring = x == 0 || z == 0 || x + 1 == side || z + 1 == side;
            for (a, &(lo, hi, state)) in runs[i].iter().enumerate() {
                let root = find(&mut parent, first[i] + a);
                grounded[root] |= i32::from(lo) <= crate::chunk::MIN_Y;
                halo[root] |= ring;
                cells[root] += 1;
                if top[root].is_none_or(|(y, _)| hi > y) {
                    top[root] = Some((hi, state));
                    at[root] = (
                        origin.0 + x as i32 * origin.2,
                        origin.1 + z as i32 * origin.2,
                    );
                }
            }
            if !ring {
                let spans = &columns[i].spans;
                for (k, span) in spans.iter().enumerate() {
                    let n = name(span.state);
                    if kind(&n) == "fluid" && (k == 0 || spans[k - 1].max < span.min) {
                        out.water_over_air += 1;
                        if std::env::var_os("HORIZON_FAR_DEBUG").is_some() {
                            println!(
                                "  fluid over air at {} {} y {}..{} {n}",
                                origin.0 + x as i32 * origin.2,
                                origin.1 + z as i32 * origin.2,
                                span.min,
                                span.max
                            );
                        }
                    }
                }
            }
        }
    }
    for root in 0..total {
        if find(&mut parent, root) != root || grounded[root] || halo[root] {
            continue;
        }
        out.floating += 1;
        out.cells += cells[root];
        let (y, state) = top[root].unwrap();
        *out.kinds.entry(kind(&name(state))).or_default() += 1;
        if std::env::var_os("HORIZON_FAR_DEBUG").is_some() {
            println!(
                "  floating at {:?} top {y} {} ({} cells)",
                at[root],
                name(state),
                cells[root]
            );
        }
    }
}

#[test]
#[ignore = "requires prepared data and a save (see the module docs)"]
fn far_floating_cells_per_level() {
    let var = |name: &str| std::env::var(name).unwrap_or_else(|_| panic!("{name}"));
    let root = PathBuf::from(var("HORIZON_TEST_RESOURCES"));
    let world = PathBuf::from(var("HORIZON_FAR_SAVE"));
    let seed: i64 = var("HORIZON_FAR_SEED").parse().unwrap();
    let at: Vec<i32> = var("HORIZON_FAR_AT")
        .split(',')
        .map(|n| n.trim().parse().unwrap())
        .collect();
    let assets = Arc::new(crate::assets::Assets::load(&root).expect("assets"));
    let biomes = Arc::new(
        Biomes::load(&horizon_assets::resource_source::ResourceRoot::directory(
            root.to_path_buf(),
        ))
        .expect("biomes"),
    );
    let registry = assets.registry;
    let item_data = crate::test_resources::item_data(
        &root,
        horizon_world::native_load_context::DataLoadOrigin::ServerDataLoad,
    )
    .expect("item data");
    let terrain = Arc::new(
        crate::vanillagen::VanillaTerrain::new(
            crate::vanillagen::TerrainResources {
                data: None,
                dir: &root.join("worldgen"),
                registry,
                biomes: &biomes,
                item_data: &item_data,
            },
            seed,
        )
        .expect("terrain"),
    );
    let save = Arc::new(SaveDir::new(
        world.join("dimensions/minecraft/overworld"),
        LevelShape::OVERWORLD,
    ));
    let generator = Arc::new(Persistent {
        inner: terrain,
        save: save.clone(),
        registry: assets.clone(),
        biomes: biomes.clone(),
    });
    let mut sampler = ChunkGenerator::lod_sampler(generator);
    let name = |state| {
        registry
            .state(state)
            .map_or_else(String::new, |s| s.name.to_string())
    };
    let limit: usize = std::env::var("HORIZON_FAR_TILES")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(48)
        .max(1);
    for level in 1..=6u8 {
        let size = 16 << level;
        let width = size / 16;
        // The ring this level is drawn in (`Plan::visit` splits a tile
        // closer than 8 of its widths).
        let (near, far) = (8 * size, 16 * size);
        let mut tiles = Vec::new();
        for tz in (at[1] - far).div_euclid(size)..=(at[1] + far).div_euclid(size) {
            for tx in (at[0] - far).div_euclid(size)..=(at[0] + far).div_euclid(size) {
                let (cx, cz) = (tx * size + size / 2, tz * size + size / 2);
                let d = (((cx - at[0]) as f64).powi(2) + ((cz - at[1]) as f64).powi(2)).sqrt();
                if d >= f64::from(near) && d < f64::from(far) {
                    tiles.push((tx, tz));
                }
            }
        }
        let step = tiles.len().div_ceil(limit).max(1);
        let mut saved = Census::default();
        let mut seeded = Census::default();
        let started = std::time::Instant::now();
        for &(tx, tz) in tiles.iter().step_by(step) {
            let (x, z) = (tx * size - width, tz * size - width);
            let columns = sampler.grid(x, z, width, 18);
            let all_saved = width <= SAVED_LOD_STRIDE
                && (x.div_euclid(16)..=(x + 18 * width - 1).div_euclid(16)).all(|cx| {
                    (z.div_euclid(16)..=(z + 18 * width - 1).div_euclid(16))
                        .all(|cz| save.lod_surfaces_shared(cx, cz, &assets, &biomes).is_some())
                });
            let out = if all_saved { &mut saved } else { &mut seeded };
            out.tiles += 1;
            count(&columns, 18, &name, out, (x, z, width));
        }
        println!(
            "level {level} (cells {width}, {near}..{far} blocks, {:.1}s): saved {saved:?} | seed {seeded:?}",
            started.elapsed().as_secs_f32()
        );
    }
}
