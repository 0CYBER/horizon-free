//! The post-processing programs of one dimension, ready for the renderer: `prepare`, `deferred`,
//! `composite` and `final` with their numbered variants, in the order they run (spec 2.1), each
//! preprocessed and translated, with the buffers it writes and the flips that follow it, and the
//! buffer settings their files carry (6.1).
//!
//! Decisions (impl-plan.md):
//! - OQ-12: a dimension's folder (`world0`, `world-1`, `world1`) replaces the base folder for that
//!   dimension when it exists; every stage file is read from it.
//! - OQ-29: a program with a geometry stage is reported and treated as missing.
//! - A program that fails to translate is reported and treated as missing (2.5).
//! - Terrain draws with the first program of its fallback chain that loads (2.3).

use std::collections::BTreeSet;

use crate::config::{self, Config};
use crate::custom::Custom;
use crate::{GbuffersInputs, Macros, Pack, Preprocessed, Program, Properties, Target};
use crate::{eval_condition, preprocess, translate_composite, translate_gbuffers};

/// Which group of 2.1 a program belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Prepare,
    Deferred,
    Composite,
    Final,
    /// A gbuffers program: the game draws its geometry with it, before deferred.
    Gbuffers,
    /// A gbuffers program drawing translucent geometry, after deferred (2.1).
    Translucent,
}

/// A blend factor of `blend.<program>` (3.2), as GL names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Factor {
    Zero,
    One,
    SrcColor,
    OneMinusSrcColor,
    DstColor,
    OneMinusDstColor,
    SrcAlpha,
    OneMinusSrcAlpha,
    DstAlpha,
    OneMinusDstAlpha,
    SrcAlphaSaturate,
}

impl Factor {
    fn parse(word: &str) -> Option<Self> {
        Some(match word {
            "ZERO" => Self::Zero,
            "ONE" => Self::One,
            "SRC_COLOR" => Self::SrcColor,
            "ONE_MINUS_SRC_COLOR" => Self::OneMinusSrcColor,
            "DST_COLOR" => Self::DstColor,
            "ONE_MINUS_DST_COLOR" => Self::OneMinusDstColor,
            "SRC_ALPHA" => Self::SrcAlpha,
            "ONE_MINUS_SRC_ALPHA" => Self::OneMinusSrcAlpha,
            "DST_ALPHA" => Self::DstAlpha,
            "ONE_MINUS_DST_ALPHA" => Self::OneMinusDstAlpha,
            "SRC_ALPHA_SATURATE" => Self::SrcAlphaSaturate,
            _ => return None,
        })
    }
}

/// Source and destination factors for colour, then for alpha.
pub type Blend = [Factor; 4];

/// The default blend of translucent geometry (3.2).
pub const TRANSLUCENT_BLEND: Blend = [
    Factor::SrcAlpha,
    Factor::OneMinusSrcAlpha,
    Factor::One,
    Factor::OneMinusSrcAlpha,
];

/// `gbuffers_water` and its fallbacks (2.3).
const WATER: [&str; 5] = [
    "gbuffers_water",
    "gbuffers_terrain",
    "gbuffers_textured_lit",
    "gbuffers_textured",
    "gbuffers_basic",
];

/// `gbuffers_terrain` and its fallbacks (2.3).
const TERRAIN: [&str; 4] = [
    "gbuffers_terrain",
    "gbuffers_textured_lit",
    "gbuffers_textured",
    "gbuffers_basic",
];

/// `gbuffers_skybasic` and its fallback (2.3).
const SKY: [&str; 2] = ["gbuffers_skybasic", "gbuffers_basic"];

/// `gbuffers_skytextured` and its fallbacks (2.3).
const SKY_TEXTURED: [&str; 3] = [
    "gbuffers_skytextured",
    "gbuffers_textured",
    "gbuffers_basic",
];

/// `gbuffers_entities` and its fallbacks (2.3).
const ENTITIES: [&str; 4] = [
    "gbuffers_entities",
    "gbuffers_textured_lit",
    "gbuffers_textured",
    "gbuffers_basic",
];

/// `gbuffers_block` and its fallbacks (2.3).
const BLOCK: [&str; 5] = [
    "gbuffers_block",
    "gbuffers_terrain",
    "gbuffers_textured_lit",
    "gbuffers_textured",
    "gbuffers_basic",
];

/// `gbuffers_entities_translucent` and its fallbacks (2.3).
const ENTITIES_TRANSLUCENT: [&str; 5] = [
    "gbuffers_entities_translucent",
    "gbuffers_entities",
    "gbuffers_textured_lit",
    "gbuffers_textured",
    "gbuffers_basic",
];

/// `gbuffers_particles` and its fallbacks (2.3).
const PARTICLES: [&str; 4] = [
    "gbuffers_particles",
    "gbuffers_textured_lit",
    "gbuffers_textured",
    "gbuffers_basic",
];

/// The sun and moon's default blend: added to the sky, as vanilla draws them.
pub const CELESTIAL_BLEND: Blend = [
    Factor::SrcAlpha,
    Factor::One,
    Factor::One,
    Factor::OneMinusSrcAlpha,
];

const GROUPS: [(Group, &str); 4] = [
    (Group::Prepare, "prepare"),
    (Group::Deferred, "deferred"),
    (Group::Composite, "composite"),
    (Group::Final, "final"),
];

pub struct PostProgram {
    /// The program's name (`composite3`).
    pub name: String,
    pub group: Group,
    pub program: Program,
    /// colortex buffer of each fragment output location (location n writes `buffers[n]`).
    pub buffers: Vec<u32>,
    /// `flip.<name>.<buffer>`: buffers forced to flip (true) or kept (false) after this program.
    pub flips: Vec<(u32, bool)>,
    /// How a gbuffers program's output blends into its buffers; `None` writes them.
    pub blend: Option<Blend>,
    /// Bit n: `colortex<n>MipmapEnabled` in its fragment stage, so buffer n's mip chain is
    /// regenerated right before it (6.1.7; not for gbuffers programs).
    pub mipmaps: u16,
}

pub struct Post {
    /// The folder the programs came from: `""` for `shaders` itself, else `world0`...
    pub folder: String,
    pub programs: Vec<PostProgram>,
    /// The flips of the virtual `deferred_pre` and `composite_pre` (6.1.6).
    pub deferred_pre: Vec<(u32, bool)>,
    pub composite_pre: Vec<(u32, bool)>,
    /// What draws solid and cutout terrain, when the game asked for it and one loads.
    pub terrain: Option<PostProgram>,
    /// What draws translucent terrain (water, stained glass) after deferred.
    pub water: Option<PostProgram>,
    /// What draws the sky (disc, sunset, stars, void) before terrain.
    pub sky: Option<PostProgram>,
    /// What draws the sun and moon, between the sunset and the stars.
    pub sky_textured: Option<PostProgram>,
    /// What draws entities, after terrain.
    pub entities: Option<PostProgram>,
    /// What draws block entities, after terrain.
    pub block: Option<PostProgram>,
    /// What draws translucent entities (name tags, shadows), after the others.
    pub entities_translucent: Option<PostProgram>,
    /// What draws particles.
    pub particles: Option<PostProgram>,
    /// Translucent entities draw after deferred (`separateEntityDraws`, 3.2).
    pub translucent_late: bool,
    /// Particles draw after deferred (`particles.ordering`, `particles.before.deferred`, 3.2).
    pub particles_late: bool,
    /// The custom uniforms of `shaders.properties` (3.4).
    pub custom: Custom,
    pub config: Config,
    /// Programs skipped and why, as `program: message` or `file:line: message`.
    pub notes: Vec<String>,
}

/// The folder a dimension reads its programs from: `world<id>` when the pack has it, else the
/// base folder `""` (1.2). `id` is the legacy number: 0 the Overworld, -1 the Nether, 1 the End.
pub fn dimension_folder(pack: &Pack, id: i32) -> String {
    let folder = format!("world{id}");
    if pack.list("").iter().any(|f| f == &folder) || !pack.list(&folder).is_empty() {
        folder
    } else {
        String::new()
    }
}

/// The game's geometry a pack may draw with its gbuffers programs: each kind's vertex inputs,
/// when the game can draw it with a pack.
#[derive(Clone, Debug, Default)]
pub struct GameInputs {
    pub terrain: Option<GbuffersInputs>,
    pub sky: Option<GbuffersInputs>,
    pub celestial: Option<GbuffersInputs>,
    /// Entities and block entities (one vertex format).
    pub entities: Option<GbuffersInputs>,
}

/// Loads the post-processing programs of `folder` and, for the geometry `inputs` describes, the
/// gbuffers programs it draws with.
pub fn load_post(
    pack: &Pack,
    folder: &str,
    properties: &Properties,
    macros: &Macros,
    inputs: &GameInputs,
) -> Post {
    let mut post = Post {
        folder: folder.to_owned(),
        programs: Vec::new(),
        deferred_pre: flips(properties, "deferred_pre"),
        composite_pre: flips(properties, "composite_pre"),
        terrain: None,
        water: None,
        sky: None,
        sky_textured: None,
        entities: None,
        block: None,
        entities_translucent: None,
        particles: None,
        translucent_late: false,
        particles_late: false,
        custom: Custom::parse(properties),
        config: Config::default(),
        notes: Vec::new(),
    };
    let mut loader = Loader {
        pack,
        folder,
        properties,
        macros,
        switches: crate::switches_on(pack),
        post: &mut post,
    };
    for (group, base) in GROUPS {
        let names: Vec<String> = if group == Group::Final {
            vec![base.to_owned()]
        } else {
            std::iter::once(base.to_owned())
                .chain((1..=99).map(|n| format!("{base}{n}")))
                .collect()
        };
        let target = match group {
            Group::Final => Target::Window,
            _ => Target::Buffers,
        };
        for name in names {
            if let Some(p) = loader.load(name, group, |v, f| translate_composite(v, f, target)) {
                loader.post.programs.push(p);
            }
        }
    }
    let mut gbuffers = |names: &[&str], group, inputs: &GbuffersInputs| {
        names.iter().find_map(|name| {
            // `alphaTest.<program>` replaces the geometry's default test (OQ-23).
            let key = properties.get(&format!("alphaTest.{name}"));
            let test = key.and_then(|v| crate::AlphaTest::parse(v.trim()));
            let mut inputs = inputs.clone();
            if let Some(test) = test {
                inputs.alpha_test = test;
            }
            loader.load(name.to_string(), group, |v, f| {
                translate_gbuffers(v, f, Target::Buffers, &inputs)
            })
        })
    };
    // The sky blends as vanilla draws its sunset fan and stars unless `blend.*` says otherwise.
    let sky = inputs
        .sky
        .as_ref()
        .and_then(|i| gbuffers(&SKY, Group::Gbuffers, i));
    let sky = sky.map(|mut p| {
        p.blend = blend(properties, &p.name).unwrap_or(Some(TRANSLUCENT_BLEND));
        p
    });
    let celestial = inputs.celestial.as_ref();
    let sky_textured = celestial.and_then(|i| gbuffers(&SKY_TEXTURED, Group::Gbuffers, i));
    let sky_textured = sky_textured.map(|mut p| {
        p.blend = blend(properties, &p.name).unwrap_or(Some(CELESTIAL_BLEND));
        p
    });
    // Entities and block entities draw without blending, as terrain does and as vanilla's
    // cutout entity types do, unless `blend.*` says otherwise (OQ-23).
    let entity_blend = |p: Option<PostProgram>| {
        p.map(|mut p| {
            p.blend = blend(properties, &p.name).flatten();
            p
        })
    };
    let (entities, block) = match &inputs.entities {
        Some(i) => (
            entity_blend(gbuffers(&ENTITIES, Group::Gbuffers, i)),
            entity_blend(gbuffers(&BLOCK, Group::Gbuffers, i)),
        ),
        None => (None, None),
    };
    // Translucent entities and particles blend (OQ-23). Particles keep vanilla's alpha cut at
    // 0.1; translucent entities lose only fully transparent texels (a shadow fades out).
    let translucent =
        |names: &[&str],
         cut: f32,
         gbuffers: &mut dyn FnMut(&[&str], &GbuffersInputs) -> Option<PostProgram>| {
            let i = inputs.entities.as_ref()?;
            let mut i = i.clone();
            i.alpha_test = Some(crate::AlphaTest {
                op: ">",
                reference: cut,
            });
            gbuffers(names, &i).map(|mut p| {
                p.blend = blend(properties, &p.name).unwrap_or(Some(TRANSLUCENT_BLEND));
                p
            })
        };
    let mut gbuffers_dyn = |names: &[&str], i: &GbuffersInputs| gbuffers(names, Group::Gbuffers, i);
    let entities_translucent = translucent(&ENTITIES_TRANSLUCENT, 0.0, &mut gbuffers_dyn);
    let particles = translucent(&PARTICLES, 0.1, &mut gbuffers_dyn);
    if let Some(inputs) = &inputs.terrain {
        let terrain = gbuffers(&TERRAIN, Group::Gbuffers, inputs);
        // gbuffers_water falls back to gbuffers_terrain's chain (2.3).
        let water = gbuffers(&WATER, Group::Translucent, inputs);
        loader.post.terrain = terrain;
        loader.post.water = water.map(|mut p| {
            p.blend = blend(properties, &p.name).unwrap_or(Some(TRANSLUCENT_BLEND));
            p
        });
        if let Some(p) = &mut loader.post.terrain {
            p.blend = blend(properties, &p.name).flatten();
        }
    }
    loader.post.sky = sky;
    loader.post.sky_textured = sky_textured;
    loader.post.entities = entities;
    loader.post.block = block;
    loader.post.entities_translucent = entities_translucent;
    loader.post.particles = particles;
    let flag = |key: &str| properties.get(key).map(|v| v.trim() == "true");
    loader.post.translucent_late = flag("separateEntityDraws").unwrap_or(false);
    // `after` when the pack has a deferred program, else `mixed`; ours are all translucent, so
    // `mixed` draws them after deferred too.
    loader.post.particles_late = match properties.get("particles.ordering").map(str::trim) {
        Some("before") => false,
        Some(_) => true,
        None => !flag("particles.before.deferred").unwrap_or(false),
    };
    post.config = std::mem::take(&mut post.config).finish();
    post
}

struct Loader<'a> {
    pack: &'a Pack,
    folder: &'a str,
    properties: &'a Properties,
    macros: &'a Macros,
    switches: BTreeSet<String>,
    post: &'a mut Post,
}

impl Loader<'_> {
    /// The program `name` translated by `translate`, or `None` when it is missing, disabled or
    /// fails (with a note).
    fn load(
        &mut self,
        name: String,
        group: Group,
        translate: impl FnOnce(&Preprocessed, &Preprocessed) -> Result<Program, Vec<crate::Error>>,
    ) -> Option<PostProgram> {
        let path = |ext: &str| match self.folder {
            "" => format!("{name}.{ext}"),
            f => format!("{f}/{name}.{ext}"),
        };
        let (vsh, fsh) = (path("vsh"), path("fsh"));
        let notes = &mut self.post.notes;
        if !self.pack.exists(&fsh) || !self.pack.exists(&vsh) {
            return None;
        }
        if !enabled(
            self.properties,
            self.folder,
            &name,
            self.macros,
            &self.switches,
        ) {
            return None;
        }
        if self.pack.exists(&path("gsh")) {
            notes.push(format!(
                "{name}: uses a geometry stage (not supported); skipped"
            ));
            return None;
        }
        let vertex = preprocess(self.pack, &vsh, self.macros);
        let fragment = preprocess(self.pack, &fsh, self.macros);
        let problems: Vec<String> = [&vertex, &fragment]
            .iter()
            .flat_map(|pp| diagnostics(pp))
            .collect();
        if !problems.is_empty() {
            notes.extend(problems);
            notes.push(format!("{name}: skipped"));
            return None;
        }
        let program = match translate(&vertex, &fragment) {
            Ok(p) => p,
            Err(errors) => {
                notes.extend(
                    errors
                        .iter()
                        .map(|e| format!("{}:{}: {}", e.file, e.line, e.message)),
                );
                notes.push(format!("{name}: skipped"));
                return None;
            }
        };
        self.post.config.read(&vertex);
        self.post.config.read(&fragment);
        let buffers = config::draw_buffers(&fragment).unwrap_or_else(|| (0..8).collect());
        let mipmaps = match group {
            Group::Gbuffers | Group::Translucent => 0,
            _ => {
                let mut own = Config::default();
                own.read(&fragment);
                (own.buffers.iter().enumerate())
                    .filter(|b| b.1.mipmap)
                    .fold(0, |m, (i, _)| m | 1 << i)
            }
        };
        Some(PostProgram {
            flips: flips(self.properties, &name),
            name,
            group,
            program,
            buffers,
            blend: None,
            mipmaps,
        })
    }
}

fn diagnostics(pp: &Preprocessed) -> Vec<String> {
    pp.diagnostics
        .iter()
        .map(|d| {
            let file = pp
                .files
                .get(d.origin.file as usize)
                .map_or("?", String::as_str);
            format!("{file}:{}: {}", d.origin.line, d.message)
        })
        .collect()
}

/// `program.<name>.enabled` (or `program.<folder>/<name>.enabled`): a boolean expression over
/// switch options (3.3): those on by default (8.1) and the defined macros.
fn enabled(
    properties: &Properties,
    folder: &str,
    name: &str,
    macros: &Macros,
    switches: &BTreeSet<String>,
) -> bool {
    let keys = [
        format!("program.{folder}/{name}.enabled"),
        format!("program.{name}.enabled"),
    ];
    let Some(expr) = keys.iter().find_map(|k| properties.get(k)) else {
        return true;
    };
    // `true`/`false` as numbers, every other name as `defined(name)`, then the `#if` arithmetic.
    let mut condition = String::new();
    for token in crate::macros::lex(expr) {
        match token {
            "true" => condition.push('1'),
            "false" => condition.push('0'),
            t if switches.contains(t) => condition.push('1'),
            t if t.starts_with(|c: char| c.is_alphabetic() || c == '_') => {
                condition.push_str(&format!("defined({t})"));
            }
            t => condition.push_str(t),
        }
    }
    eval_condition(&condition, macros).ok() != Some(0)
}

/// `blend.<program>`: `Some(None)` for `off`, `Some(factors)`, or `None` when absent or not
/// four known factors.
fn blend(properties: &Properties, program: &str) -> Option<Option<Blend>> {
    let value = properties.get(&format!("blend.{program}"))?.trim();
    if value == "off" {
        return Some(None);
    }
    let factors: Vec<Factor> = value
        .split_whitespace()
        .map(Factor::parse)
        .collect::<Option<_>>()?;
    Some(Some(factors.try_into().ok()?))
}

/// `flip.<program>.<buffer>=<bool>` keys of one program.
fn flips(properties: &Properties, program: &str) -> Vec<(u32, bool)> {
    let prefix = format!("flip.{program}.");
    properties
        .entries
        .iter()
        .filter_map(|e| {
            let buffer = config::buffer_index(e.key.strip_prefix(&prefix)?)?;
            let value = match e.value.trim() {
                "true" => true,
                "false" => false,
                _ => return None,
            };
            Some((buffer as u32, value))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack::tests::zip;

    const VSH: &str = "#version 120\nvarying vec2 uv;\nvoid main() {\n\
                       gl_Position = ftransform();\nuv = gl_MultiTexCoord0.xy;\n}\n";
    const FSH: &str = "#version 120\nuniform sampler2D colortex0;\nvarying vec2 uv;\n\
                       void main() {\n/* DRAWBUFFERS:3 */\n\
                       gl_FragData[0] = texture2D(colortex0, uv);\n}\n";

    #[test]
    fn programs_load_in_order_with_their_settings() {
        let pack = Pack::from_zip_bytes(
            "t",
            zip(&[
                ("shaders/composite.vsh", VSH),
                ("shaders/composite.fsh", FSH),
                ("shaders/composite2.vsh", VSH),
                ("shaders/composite2.fsh", FSH),
                ("shaders/deferred.vsh", VSH),
                (
                    "shaders/deferred.fsh",
                    "#version 120\nconst bool colortex5MipmapEnabled = true;\n\
                     void main() { gl_FragColor = vec4(1.0); }\n",
                ),
                ("shaders/composite1.fsh", FSH),
                ("shaders/composite3.vsh", VSH),
                ("shaders/composite3.fsh", FSH),
                ("shaders/composite4.vsh", VSH),
                ("shaders/composite4.fsh", FSH),
                ("shaders/composite4.gsh", "x"),
                ("shaders/composite5.vsh", VSH),
                (
                    "shaders/composite5.fsh",
                    "#version 120\nvoid main() { undefined(); }\n",
                ),
                ("shaders/final.vsh", VSH),
                ("shaders/final.fsh", FSH),
                ("shaders/world-1/final.vsh", VSH),
                ("shaders/world-1/final.fsh", FSH),
            ]),
        )
        .unwrap();
        let properties = Properties::parse(
            "program.composite3.enabled = HQ && !LOW\nflip.composite2.gaux1=true\n\
             flip.composite.colortex3 = false\nflip.composite_pre.colortex1=true\n",
            &Macros::default(),
        );
        assert_eq!(dimension_folder(&pack, 0), "");
        assert_eq!(dimension_folder(&pack, -1), "world-1");
        let post = load_post(
            &pack,
            "",
            &properties,
            &Macros::standard(),
            &GameInputs::default(),
        );
        let names: Vec<&str> = post.programs.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["deferred", "composite", "composite2", "final"]);
        let p = &post.programs;
        assert_eq!(p[0].buffers, (0..8).collect::<Vec<_>>());
        assert_eq!((p[0].mipmaps, p[1].mipmaps), (1 << 5, 0));
        assert_eq!(p[1].buffers, [3]);
        assert_eq!(p[1].flips, [(3, false)]);
        assert_eq!(p[2].flips, [(4, true)]);
        assert_eq!(post.composite_pre, [(1, true)]);
        assert_eq!(p[3].group, Group::Final);
        let notes = post.notes.join("\n");
        assert!(
            notes.contains("composite4: uses a geometry stage"),
            "{notes}"
        );
        assert!(notes.contains("composite5.fsh:2:"), "{notes}");
        assert!(notes.contains("composite5: skipped"), "{notes}");

        let mut macros = Macros::standard();
        macros.define("HQ", "");
        let post = load_post(
            &pack,
            "world-1",
            &properties,
            &macros,
            &GameInputs::default(),
        );
        let names: Vec<&str> = post.programs.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["final"]);
        let post = load_post(&pack, "", &properties, &macros, &GameInputs::default());
        assert!(post.programs.iter().any(|p| p.name == "composite3"));
    }

    /// Water falls back to the terrain chain and blends by default, as the sky does; entities
    /// and block entities fall back along theirs; `blend.*` overrides.
    #[test]
    fn water_falls_back_to_terrain_and_blends() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/packs/terrain-test");
        let pack = Pack::open(&dir).unwrap();
        let macros = Macros::standard();
        let inputs = crate::GbuffersInputs {
            declarations: "layout(location = 0) in vec3 hz_v_pos;".into(),
            body: "    hz_Vertex = vec4(hz_v_pos, 1.0);\n    hz_Color = vec4(1.0);\n".into(),
            ..Default::default()
        };
        let inputs = GameInputs {
            terrain: Some(inputs.clone()),
            sky: Some(inputs.clone()),
            celestial: Some(inputs.clone()),
            entities: Some(inputs),
        };
        let post = load_post(&pack, "", &Properties::default(), &macros, &inputs);
        let (terrain, water) = (post.terrain.unwrap(), post.water.unwrap());
        assert_eq!((terrain.group, terrain.blend), (Group::Gbuffers, None));
        assert_eq!(water.name, "gbuffers_terrain");
        let sky = post.sky.unwrap();
        assert_eq!(
            (sky.name.as_str(), sky.group, sky.blend),
            (
                "gbuffers_skybasic",
                Group::Gbuffers,
                Some(TRANSLUCENT_BLEND)
            )
        );
        // gbuffers_skytextured falls back to gbuffers_textured, then gbuffers_basic.
        let sun = post.sky_textured.unwrap();
        assert_eq!(
            (sun.name.as_str(), sun.blend),
            ("gbuffers_textured", Some(CELESTIAL_BLEND))
        );
        assert_eq!(
            (water.group, water.blend),
            (Group::Translucent, Some(TRANSLUCENT_BLEND))
        );
        // Entities fall back to gbuffers_textured, block entities to gbuffers_terrain; both
        // draw without blending.
        let (entities, block) = (post.entities.unwrap(), post.block.unwrap());
        assert_eq!(
            (entities.name.as_str(), entities.blend),
            ("gbuffers_textured", None)
        );
        assert_eq!(
            (block.name.as_str(), block.blend),
            ("gbuffers_terrain", None)
        );
        // Translucent entity parts and particles blend; particles draw after deferred unless
        // the pack says otherwise, translucent entities before unless `separateEntityDraws`.
        let (translucent, particles) = (
            post.entities_translucent.as_ref().unwrap(),
            post.particles.as_ref().unwrap(),
        );
        assert_eq!(
            (translucent.name.as_str(), translucent.blend),
            ("gbuffers_textured", Some(TRANSLUCENT_BLEND))
        );
        assert_eq!(particles.blend, Some(TRANSLUCENT_BLEND));
        assert!(post.particles_late && !post.translucent_late);
        let ordering = Properties::parse(
            "particles.ordering = before\nseparateEntityDraws = true\n",
            &macros,
        );
        let post = load_post(&pack, "", &ordering, &macros, &inputs);
        assert!(!post.particles_late && post.translucent_late);
        let properties = Properties::parse(
            "blend.gbuffers_terrain = ONE ONE ZERO ONE\nblend.gbuffers_water = off\n",
            &macros,
        );
        let terrain_blend = blend(&properties, "gbuffers_terrain");
        assert_eq!(
            terrain_blend,
            Some(Some([Factor::One, Factor::One, Factor::Zero, Factor::One]))
        );
        assert_eq!(blend(&properties, "gbuffers_water"), Some(None));
        assert_eq!(blend(&properties, "gbuffers_basic"), None);
    }

    #[test]
    fn horizons_test_pack_loads() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/packs/post-test");
        let pack = Pack::open(&dir).unwrap();
        let macros = Macros::standard();
        let properties = Properties::parse(&pack.read_text("shaders.properties").unwrap(), &macros);
        let post = load_post(
            &pack,
            &dimension_folder(&pack, 0),
            &properties,
            &macros,
            &GameInputs::default(),
        );
        assert!(post.notes.is_empty(), "{:?}", post.notes);
        let names: Vec<&str> = post.programs.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["composite", "composite1", "final"]);
        assert_eq!(post.programs[0].buffers, [0, 2]);
        assert_eq!(post.programs[1].buffers, [0]);
        assert_eq!(
            post.config.buffers[2].format,
            crate::Format::parse("RGBA16F").unwrap()
        );
    }
}
