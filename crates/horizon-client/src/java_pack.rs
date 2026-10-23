//! Java (OptiFine/Iris) shader packs in `shaderpacks/` (docs/shader-packs.md):
//! a folder holding `shaders/`, or a `.zip`. `horizon-shaderpack` translates
//! their full-screen programs (`begin`, `prepare`, `deferred`, `composite`,
//! `final`) to WGSL, and `gfx/java_stages.rs` runs them over Horizon's world
//! as Iris runs them over its own. Geometry programs (`gbuffers_*`,
//! `shadow`) don't run yet: the world is drawn by Horizon.
use horizon_shaderpack::{
    JavaPack,
    pipeline::{Directives, Program, TargetSettings},
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
    /// world's colour and depth enter `world_target` and `depthtex*` after them.
    pub world_at: usize,
    /// The colour target the world goes into, where the pack's terrain
    /// program draws (MakeUp: colortex1).
    pub world_target: u32,
    /// The targets whose alternate side the world is written to.
    pub flipped_after_prepare: BTreeSet<u32>,
    pub final_pass: Option<Program>,
    /// The targets whose alternate side holds the frame's last write.
    pub flipped_at_end: BTreeSet<u32>,
    /// `sunPathRotation`, `eyeBrightnessHalflife`, `wetnessHalflife` and
    /// `drynessHalflife`.
    pub directives: Directives,
}

/// The key of `texture.noise` (`noisetex`) in [`JavaPasses::custom_textures`].
pub const NOISE_KEY: &str = "noise";

/// The key of `texture.<stage>.<sampler>` in [`JavaPasses::custom_textures`]
/// for a pass of `program` (`world0/composite2`): Iris's texture stages, where
/// `final` reads the `composite` ones.
pub fn texture_key(program: &str, sampler: &str) -> String {
    let name = program.rsplit('/').next().unwrap_or(program);
    let stage = match name.trim_end_matches(|c: char| c.is_ascii_digit()) {
        "final" => "composite",
        stage => stage,
    };
    format!("{stage}.{sampler}")
}

/// A loaded Java pack.
pub struct JavaPasses {
    pub folders: Vec<JavaFolder>,
    /// `uniform.*` / `variable.*` from `shaders.properties`.
    pub uniforms: CustomUniforms,
    /// What the pack keeps of the vanilla sky (`clouds`, `sun`, `moon`).
    pub sky: SkyDirectives,
    /// `texture.<stage>.<sampler>` and `texture.noise` images, by
    /// `"<stage>.<sampler>"` and [`NOISE_KEY`].
    pub custom_textures: BTreeMap<String, crate::shader_pack::Rgba>,
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
        folders.push(JavaFolder {
            folder: name.to_owned(),
            targets: plan.targets,
            world_at: early.count(),
            world_target: plan.world_target,
            passes,
            flipped_after_prepare: plan.flipped_after_prepare,
            final_pass,
            flipped_at_end: plan.flipped_at_end,
            directives: plan.directives,
        });
    }
    if folders.is_empty() {
        return Err("no deferred, composite or final programs".into());
    }
    let mut custom_textures = BTreeMap::new();
    for tex in properties.custom_textures() {
        let key = match tex.stage {
            Some(stage) => format!("{stage}.{}", tex.sampler),
            None if tex.sampler == NOISE_KEY => NOISE_KEY.to_owned(),
            None => continue,
        };
        // Only PNGs for now: resource locations and raw `.dat` textures are
        // skipped and the sampler reads what it would without them.
        let image = pack
            .files
            .read_bytes(&tex.path)
            .ok_or_else(|| "not in the pack".to_owned())
            .and_then(|bytes| crate::shader_pack::decode_png(&bytes));
        match image {
            Ok(image) => {
                custom_textures.insert(key, image);
            }
            Err(e) => log::warn!("Horizon: shader pack texture {}: {e}", tex.path),
        }
    }
    let uniforms = properties.custom_uniforms();
    for problem in &uniforms.problems {
        log::warn!("Horizon: shader pack custom uniform {problem}");
    }
    Ok(JavaPasses {
        folders,
        uniforms,
        sky: properties.sky(),
        custom_textures,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_textures_belong_to_iris_s_stages() {
        assert_eq!(texture_key("world0/composite2", "gaux1"), "composite.gaux1");
        assert_eq!(texture_key("final", "colortex4"), "composite.colortex4");
        assert_eq!(
            texture_key("world-1/deferred10", "noisetex"),
            "deferred.noisetex"
        );
        assert_eq!(texture_key("prepare", "depthtex2"), "prepare.depthtex2");
    }

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
                Ok(pack) => {
                    let folders: Vec<String> = pack
                        .folders
                        .iter()
                        .map(|f| {
                            let n = f.passes.len() + usize::from(f.final_pass.is_some());
                            format!("{} {n} passes", f.folder)
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
