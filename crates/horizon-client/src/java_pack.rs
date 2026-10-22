//! Java (OptiFine/Iris) shader packs in `shaderpacks/` (docs/shader-packs.md):
//! a folder holding `shaders/`, or a `.zip`. `horizon-shaderpack` translates
//! their full-screen programs (`begin`, `prepare`, `deferred`, `composite`,
//! `final`) to WGSL, and `gfx/java_stages.rs` runs them over Horizon's world
//! as Iris runs them over its own. Of the geometry programs, the terrain's
//! (`gbuffers_terrain`, `gbuffers_water` and their fallbacks) draw Horizon's
//! terrain (`gfx/java_terrain.rs`); the rest of the world is Horizon's.
use horizon_shaderpack::{
    JavaPack,
    pipeline::{Directives, GeometryProgram, Program, TargetSettings},
    preprocess::Environment,
    properties::SkyDirectives,
    uniforms::CustomUniforms,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

/// The passes of one program folder (`world0`, or `""` for the pack's root).
pub struct JavaFolder {
    pub folder: String,
    pub targets: Vec<TargetSettings>,
    /// `begin*`, `prepare*`, `deferred*`, `composite*`, in order.
    pub passes: Vec<Program>,
    /// How many of `passes` run before the world (`begin*`, `prepare*`): the
    /// world's colour and depth enter `colortex0` and `depthtex*` after them.
    pub world_at: usize,
    /// The targets whose alternate side the world is written to.
    pub flipped_after_prepare: BTreeSet<u32>,
    pub final_pass: Option<Program>,
    /// The targets whose alternate side holds the frame's last write.
    pub flipped_at_end: BTreeSet<u32>,
    /// `sunPathRotation` and `eyeBrightnessHalflife`.
    pub directives: Directives,
    /// The programs Horizon's solid and translucent terrain are drawn with,
    /// each translated for instanced and flat draws (`variants[0]`).
    pub terrain: Option<GeometryProgram>,
    pub water: Option<GeometryProgram>,
    /// How many of `passes` run before translucent terrain (`begin*`,
    /// `prepare*`, `deferred*`).
    pub translucent_at: usize,
    pub flipped_after_translucent: BTreeSet<u32>,
    /// Each draw buffer's blending in `terrain` and `water`.
    pub terrain_blends: Vec<Option<wgpu::BlendState>>,
    pub water_blends: Vec<Option<wgpu::BlendState>>,
    /// The shadow map's programs over Horizon's solid and translucent
    /// terrain (instanced draws only), drawn after `begin*`, into
    /// `shadowcolor` targets set by `shadow_targets`.
    pub shadow: Option<GeometryProgram>,
    pub shadow_water: Option<GeometryProgram>,
    pub shadow_targets: Vec<TargetSettings>,
    pub shadow_blends: Vec<Option<wgpu::BlendState>>,
    pub shadow_water_blends: Vec<Option<wgpu::BlendState>>,
    /// How many of `passes` run before the shadow map (`begin*`).
    pub shadow_at: usize,
}

/// A loaded Java pack.
pub struct JavaPasses {
    pub folders: Vec<JavaFolder>,
    /// `uniform.*` / `variable.*` from `shaders.properties`.
    pub uniforms: CustomUniforms,
    /// What the pack keeps of the vanilla sky (`clouds`, `sun`, `moon`).
    pub sky: SkyDirectives,
    /// `block.properties`: block name → `mc_Entity` id.
    pub block_ids: BTreeMap<String, i32>,
}

impl JavaPasses {
    /// The folder Iris uses in `dimension` (0 overworld, -1 the Nether, 1 the
    /// End): `world<N>` when the pack has one, else `world0`, else the root.
    pub fn folder_for(&self, dimension: i8) -> Option<usize> {
        let find = |name: &str| self.folders.iter().position(|f| f.folder == name);
        find(&format!("world{dimension}"))
            .or_else(|| find("world0"))
            .or_else(|| find(""))
    }
}

/// A folder holding `shaders/`, or a zip.
pub fn is_java_pack(path: &Path) -> bool {
    path.join("shaders").is_dir()
        || path.is_file()
            && path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("zip"))
}

/// The `MC_GL_VENDOR_*` name of a wgpu adapter (`AdapterInfo::vendor`, a
/// PCI id; Apple's GPUs report 0).
pub fn vendor_name(vendor: u32, adapter: &str) -> &'static str {
    match vendor {
        0x10de => "NVIDIA Corporation",
        0x1002 | 0x1022 => "AMD",
        0x8086 => "Intel",
        _ if adapter.starts_with("Apple") => "Apple",
        _ => "Unknown",
    }
}

/// Open the pack at `path` and translate its full-screen passes with the
/// player's `values`; `vendor` and `renderer` name the GPU to the pack.
pub fn load(
    path: &Path,
    values: &BTreeMap<String, String>,
    vendor: &str,
    renderer: &str,
) -> Result<JavaPasses, String> {
    let pack = JavaPack::open(path)?;
    let values = pack.values(values);
    let env = Environment::iris(vendor, renderer, 4);
    let properties = pack.properties(&values, &env)?;
    let mut names: Vec<&str> = ["world0", "world-1", "world1"]
        .into_iter()
        .filter(|d| {
            pack.programs
                .iter()
                .any(|p| p.starts_with(&format!("{d}/")))
        })
        .collect();
    if pack.programs.iter().any(|p| !p.contains('/')) {
        names.push("");
    }
    let mut folders = Vec::new();
    for name in names {
        let plan = pack.passes(name, &values, &env, &properties)?;
        let compile = |pass| {
            let program = pack.compile(pass, &plan.targets, &values, &env, &properties)?;
            if let Some((image, _)) = program.fragment.interface.images.first() {
                return Err(format!(
                    "{}: images ({image}) aren't supported yet",
                    program.pass.program
                ));
            }
            Ok::<_, String>(program)
        };
        let early = plan.begin.iter().chain(&plan.prepare);
        let passes = early
            .clone()
            .chain(&plan.deferred)
            .chain(&plan.composite)
            .map(compile)
            .collect::<Result<Vec<_>, _>>()?;
        let final_pass = plan.final_pass.as_ref().map(compile).transpose()?;
        if passes.is_empty() && final_pass.is_none() {
            continue;
        }
        // A geometry program that doesn't translate leaves that terrain to
        // Horizon, with a warning.
        let geometry_with = |pass: &Option<horizon_shaderpack::pipeline::Pass>,
                             alpha_test,
                             flat: &[bool]| {
            let pass = pass.as_ref()?;
            let glues: Vec<_> = flat
                .iter()
                .map(|&flat| crate::gfx::java_terrain::glue(flat, alpha_test))
                .collect();
            match pack.compile_geometry(pass, &plan.targets, &values, &env, &properties, &glues) {
                Ok(program) => Some(program),
                Err(e) => {
                    log::warn!("Horizon: shader pack {}: {e}", pass.program);
                    None
                }
            }
        };
        let geometry = |pass: &Option<horizon_shaderpack::pipeline::Pass>, alpha_test| {
            geometry_with(pass, alpha_test, &[false, true])
        };
        // Iris's alpha test of cutout terrain (`AlphaTests.ONE_TENTH_ALPHA`).
        let terrain = geometry(&plan.terrain, Some(0.1));
        let water = terrain.as_ref().and_then(|_| geometry(&plan.water, None));
        // The shadow map draws only with the pack's terrain, and only from
        // instanced draws (the flat draws hold what the camera sees).
        let shadow = terrain
            .as_ref()
            .and_then(|_| geometry_with(&plan.shadow, Some(0.1), &[false]));
        let shadow_water = shadow
            .as_ref()
            .and_then(|_| geometry_with(&plan.shadow_water, None, &[false]));
        let translucent_at = early.clone().count() + plan.deferred.len();
        let blends = |g: &Option<GeometryProgram>, default| {
            g.as_ref().map_or_else(Vec::new, |g| {
                geometry_blends(
                    &properties,
                    &g.program.pass.program,
                    &g.program.draw_buffers,
                    default,
                )
            })
        };
        let terrain_blends = blends(&terrain, None);
        // Iris's shadow programs don't blend (`BlendModeOverride.OFF`).
        let shadow_blends = blends(&shadow, None);
        let shadow_water_blends = blends(&shadow_water, None);
        // Vanilla's translucent blending (`BlendFunction.TRANSLUCENT`).
        let water_blends = blends(
            &water,
            Some(wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::SrcAlpha,
                    dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                    operation: wgpu::BlendOperation::Add,
                },
            }),
        );
        folders.push(JavaFolder {
            folder: name.to_owned(),
            targets: plan.targets,
            world_at: early.count(),
            passes,
            flipped_after_prepare: plan.flipped_after_prepare,
            final_pass,
            flipped_at_end: plan.flipped_at_end,
            directives: plan.directives,
            terrain,
            water,
            translucent_at,
            flipped_after_translucent: plan.flipped_after_translucent,
            terrain_blends,
            water_blends,
            shadow,
            shadow_water,
            shadow_targets: plan.shadow_targets,
            shadow_blends,
            shadow_water_blends,
            shadow_at: plan.begin.len(),
        });
    }
    if folders.is_empty() {
        return Err("no deferred, composite or final programs".into());
    }
    let uniforms = properties.custom_uniforms();
    for problem in &uniforms.problems {
        log::warn!("Horizon: shader pack custom uniform {problem}");
    }
    let block_ids = pack.block_ids(&values, &env).unwrap_or_else(|e| {
        log::warn!("Horizon: shader pack {e}");
        BTreeMap::new()
    });
    Ok(JavaPasses {
        folders,
        uniforms,
        sky: properties.sky(),
        block_ids,
    })
}

/// A geometry program's blending per draw buffer: `default`, or the
/// pack's `blend.<program>` and `blend.<program>.<buffer>` (Iris's
/// `BlendModeOverride`; the folder isn't part of the name).
fn geometry_blends(
    properties: &horizon_shaderpack::properties::ShaderProperties,
    program: &str,
    draw_buffers: &[u32],
    default: Option<wgpu::BlendState>,
) -> Vec<Option<wgpu::BlendState>> {
    use horizon_shaderpack::properties::Blend;
    let name = program.rsplit('/').next().unwrap_or(program);
    let state = |blend: &Result<Blend, String>| match blend {
        Ok(Blend::Off) => Some(None),
        Ok(Blend::Factors(f)) => {
            let [a, b, c, d] = f.each_ref().map(|f| blend_factor(f));
            Some(Some(wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: a?,
                    dst_factor: b?,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent {
                    src_factor: c?,
                    dst_factor: d?,
                    operation: wgpu::BlendOperation::Add,
                },
            }))
        }
        Err(e) => {
            log::warn!("Horizon: shader pack blend.{name}: {e}");
            None
        }
    };
    let all = properties
        .blend(name)
        .and_then(|b| state(&b))
        .unwrap_or(default);
    let buffers = properties.buffer_blends(name);
    draw_buffers
        .iter()
        .map(|i| {
            buffers
                .iter()
                .find(|(b, _)| b == i)
                .and_then(|(_, b)| state(b))
                .unwrap_or(all)
        })
        .collect()
}

/// A GL blend factor's wgpu one.
fn blend_factor(name: &str) -> Option<wgpu::BlendFactor> {
    use wgpu::BlendFactor as F;
    Some(match name {
        "ZERO" => F::Zero,
        "ONE" => F::One,
        "SRC_COLOR" => F::Src,
        "ONE_MINUS_SRC_COLOR" => F::OneMinusSrc,
        "DST_COLOR" => F::Dst,
        "ONE_MINUS_DST_COLOR" => F::OneMinusDst,
        "SRC_ALPHA" => F::SrcAlpha,
        "ONE_MINUS_SRC_ALPHA" => F::OneMinusSrcAlpha,
        "DST_ALPHA" => F::DstAlpha,
        "ONE_MINUS_DST_ALPHA" => F::OneMinusDstAlpha,
        "SRC_ALPHA_SATURATE" => F::SrcAlphaSaturated,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_the_full_screen_passes_of_a_java_pack() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../horizon-shaderpack/tests/fixtures/passes");
        assert!(is_java_pack(&dir));
        let pack = load(&dir, &BTreeMap::new(), "Apple", "Apple M4 Pro").unwrap();
        let [folder] = &pack.folders[..] else {
            panic!("one folder, the root");
        };
        assert_eq!(folder.folder, "");
        assert_eq!(
            folder
                .passes
                .iter()
                .map(|p| p.pass.program.as_str())
                .collect::<Vec<_>>(),
            ["deferred", "composite", "composite1"]
        );
        assert_eq!(folder.world_at, 0);
        assert!(folder.final_pass.is_some());
        // The overworld, the Nether and the End all use the root.
        assert_eq!([-1, 0, 1].map(|d| pack.folder_for(d)), [Some(0); 3]);
        assert_eq!(
            vendor_name(0x10de, "NVIDIA GeForce RTX 4080"),
            "NVIDIA Corporation"
        );
        assert_eq!(vendor_name(0, "Apple M4 Pro"), "Apple");
    }

    /// wgpu is given the WGSL text: it must parse again.
    fn reparse_error(pack: &JavaPasses) -> Option<String> {
        let passes = pack
            .folders
            .iter()
            .flat_map(|f| f.passes.iter().chain(&f.final_pass));
        let geometry = pack.folders.iter().flat_map(|f| {
            f.terrain
                .iter()
                .chain(&f.water)
                .chain(&f.shadow)
                .chain(&f.shadow_water)
        });
        passes
            .flat_map(|p| [(p, &p.vertex, "vsh"), (p, &p.fragment, "fsh")])
            .chain(geometry.flat_map(|g| {
                let p = &g.program;
                [(p, &p.vertex, "vsh"), (p, &p.fragment, "fsh")]
                    .into_iter()
                    .chain(g.variants.iter().map(move |v| (p, v, "vsh (flat)")))
            }))
            .find_map(|(p, t, ext)| {
                let e = naga::front::wgsl::parse_str(&t.wgsl).err()?;
                Some(format!(
                    "{}.{ext}: {}",
                    p.pass.program,
                    e.emit_to_string(&t.wgsl)
                ))
            })
    }

    /// The packs in `HORIZON_TEST_SHADERPACKS` (folders or zips) load as the
    /// game loads them, with their default options: the compatibility table
    /// of docs/shader-packs.md. Skipped when the variable isn't set.
    #[test]
    fn real_packs_load() {
        let Some(dir) = std::env::var_os("HORIZON_TEST_SHADERPACKS") else {
            return;
        };
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| is_java_pack(p))
            .collect();
        entries.sort();
        let mut failures = Vec::new();
        for path in entries {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            match load(&path, &BTreeMap::new(), "Apple", "Apple M4 Pro") {
                Ok(pack) if let Some(e) = reparse_error(&pack) => {
                    println!("{name}: {e}");
                    failures.push(name);
                }
                Ok(pack) => {
                    let folders: Vec<String> = pack
                        .folders
                        .iter()
                        .map(|f| {
                            let n = f.passes.len() + usize::from(f.final_pass.is_some());
                            let geometry: Vec<&str> = f
                                .terrain
                                .iter()
                                .chain(&f.water)
                                .chain(&f.shadow)
                                .chain(&f.shadow_water)
                                .map(|g| g.program.pass.program.as_str())
                                .collect();
                            format!("{} {n} passes, terrain {geometry:?}", f.folder)
                        })
                        .collect();
                    println!("{name}: loads ({})", folders.join(", "));
                }
                Err(e) => {
                    println!("{name}: {e}");
                    failures.push(name);
                }
            }
        }
        assert!(failures.is_empty(), "{failures:?}");
    }
}
