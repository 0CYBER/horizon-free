//! The rule for new names (docs/api/shaders-reference-0.2.md, "Room to grow"): what Horizon puts
//! before a 0.2 pack's files gains a name only where the pack asked for it, by a manifest key or
//! by a stage added later. A pack names its line (`api = "0.2"`), not the additions it was
//! written against, so a name added to a prelude it already has would break the pack that uses
//! that name for something of its own.
//!
//! `tests/names-0.2/<stage>.txt` lists every name each stage's prelude declares for a pack that
//! sets no key bringing more. To add a name to a stage a released pack may have, put it behind a
//! manifest key (and compose the pack here without it), or add its line to the list in a pull
//! request that says why no pack can hold the name. A stage added later starts its own list:
//! no pack had it, so its names collide with nothing.
use horizon_shader_api::{MANIFEST, Pack, Problem, StageKind, check, compose, compose_scene};
use std::collections::BTreeSet;

const STAGES: &str = "[sky]\nfile = \"sky.wgsl\"\n[material]\nfile = \"material.wgsl\"\n\
                      [vertex]\nfile = \"vertex.wgsl\"\n[weather]\nfile = \"weather.wgsl\"\n\
                      [[post]]\nfile = \"post.wgsl\"\n[tonemap]\nfile = \"tonemap.wgsl\"\n\
                      [entity]\nfile = \"entity.wgsl\"\n";

/// Each stage written as plainly as it can be, by its file.
const FILES: [(&str, &str); 7] = [
    (
        "sky.wgsl",
        "fn sky(input: SkyInput) -> vec4<f32> { return input.color; }\n",
    ),
    (
        "material.wgsl",
        "fn material(input: MaterialInput) -> Surface { return Surface(input.albedo); }\n",
    ),
    (
        "vertex.wgsl",
        "fn vertex(input: VertexInput) -> vec3<f32> { return input.position; }\n",
    ),
    (
        "weather.wgsl",
        "fn weather(input: WeatherInput) -> vec4<f32> { return input.color; }\n",
    ),
    (
        "post.wgsl",
        "fn post(input: PostInput) -> vec4<f32> { return input.color; }\n",
    ),
    (
        "tonemap.wgsl",
        "fn tonemap(input: TonemapInput) -> vec3<f32> { return input.color; }\n",
    ),
    (
        "entity.wgsl",
        "fn entity(input: EntityInput) -> Surface { return Surface(input.albedo); }\n",
    ),
];

/// A 0.2 pack with every stage and no other key, `replaced` standing in for one of its files.
fn pack(replaced: Option<(&str, &str)>) -> Pack {
    pack_with("", replaced)
}

/// `pack` with `keys` added to its manifest.
fn pack_with(keys: &str, replaced: Option<(&str, &str)>) -> Pack {
    let manifest = format!(
        "schema = 1\nid = \"p\"\nname = \"P\"\nversion = \"1\"\napi = \"0.2\"\n{STAGES}{keys}"
    );
    let mut files: std::collections::BTreeMap<String, Vec<u8>> = FILES
        .iter()
        .map(|(name, text)| ((*name).to_owned(), text.as_bytes().to_vec()))
        .collect();
    files.insert(MANIFEST.into(), manifest.into_bytes());
    if let Some((name, text)) = replaced {
        files.insert(name.into(), text.as_bytes().to_vec());
    }
    Pack::from_files(files).unwrap()
}

/// Every name `wgsl` declares at its top level, read from the text so that nothing a parser
/// folds away (an alias) is missed: the identifier after `fn`, `struct`, `alias`, `const`,
/// `override` or `var`, wherever the lines break.
fn declared(wgsl: &str) -> BTreeSet<String> {
    let text: String = wgsl
        .lines()
        .map(|line| line.split("//").next().unwrap())
        .flat_map(|line| line.chars().chain(['\n']))
        .collect();
    let text = text.replace('{', " { ").replace('}', " } ");
    let mut tokens = text
        .split(|c: char| !(c.is_ascii_alphanumeric() || "_{}".contains(c)))
        .filter(|t| !t.is_empty());
    let mut names = BTreeSet::new();
    let mut depth = 0usize;
    while let Some(token) = tokens.next() {
        match token {
            "{" => depth += 1,
            "}" => depth -= 1,
            "fn" | "struct" | "alias" | "const" | "override" | "var" if depth == 0 => {
                // `var<uniform> frame`, `var<private> x`: the address space comes first.
                let name = tokens
                    .by_ref()
                    .find(|t| !["uniform", "private", "storage", "read", "read_write"].contains(t));
                names.extend(name.map(str::to_owned));
            }
            _ => {}
        }
    }
    names
}

/// The scan finds what naga folds away, however it is laid out.
#[test]
fn declared_reads_aliases_across_lines() {
    let found = declared(
        "alias\n  Wide = vec4<f32>; alias Thin = f32; // alias Not = u32;\nfn f() { const inner = 1; }\n",
    );
    assert_eq!(
        found,
        BTreeSet::from(["Wide", "Thin", "f"].map(str::to_owned))
    );
}

/// What naga finds in the same text, to hold `declared` to: it must miss nothing naga sees. The
/// material prelude's LabPBR functions call into the renderer (a traced post prelude has its own).
fn parsed(wgsl: &str) -> BTreeSet<String> {
    let pbr = match wgsl.contains("fn horizon_block_pbr(") {
        true => "",
        false => horizon_shader_api::stages::PBR_STUB,
    };
    let text = format!("{wgsl}{pbr}");
    let m = naga::front::wgsl::parse_str(&text).unwrap();
    let types = m.types.iter().filter_map(|(_, t)| t.name.clone());
    let functions = m.functions.iter().filter_map(|(_, f)| f.name.clone());
    let globals = m
        .global_variables
        .iter()
        .filter_map(|(_, g)| g.name.clone());
    let constants = m.constants.iter().filter_map(|(_, c)| c.name.clone());
    let overrides = m.overrides.iter().filter_map(|(_, o)| o.name.clone());
    let all = types.chain(functions).chain(globals).chain(constants);
    all.chain(overrides).collect()
}

/// The names the prelude of `composed` declares that a pack could hold: those starting with
/// `horizon_` are Horizon's by the API's rules.
fn prelude_names(composed: &horizon_shader_api::Composed, kind: StageKind) -> BTreeSet<String> {
    let (part, range) = &composed.parts[0];
    assert_eq!(part, "<prelude>");
    let prelude = &composed.text[range.clone()];
    let have: BTreeSet<String> = declared(prelude)
        .into_iter()
        .filter(|n| !n.starts_with("horizon_"))
        .collect();
    let seen = parsed(prelude);
    // naga's own types of a ray query, which the scene's prelude uses and declares nowhere:
    // the check refuses them in the files of a pack with `[ray_tracing]`.
    let naga_s = ["RayDesc", "RayIntersection"];
    let missed: Vec<_> = seen
        .iter()
        .filter(|n| !n.starts_with("horizon_") && !have.contains(*n))
        .filter(|n| !naga_s.contains(&n.as_str()))
        .collect();
    assert!(missed.is_empty(), "{kind:?}: `declared` misses {missed:?}");
    have
}

/// `tests/names-0.2/<stage>.txt`, and where it is.
fn listed_names(kind: StageKind) -> (BTreeSet<String>, String) {
    let path = format!(
        "{}/tests/names-0.2/{}.txt",
        env!("CARGO_MANIFEST_DIR"),
        kind.name()
    );
    let list = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let listed = list
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_owned)
        .collect();
    (listed, path)
}

#[test]
fn a_0_2_prelude_declares_only_the_names_on_its_list() {
    let pack = pack(None);
    for kind in StageKind::ALL {
        let file = format!("{}.wgsl", kind.name());
        let have = prelude_names(&compose(&pack, kind, &file), kind);
        let (listed, path) = listed_names(kind);
        let new: Vec<_> = have.difference(&listed).collect();
        assert!(
            new.is_empty(),
            "the {} stage's 0.2 prelude now declares {new:?}: a pack written before may use that name for \
             something of its own and would stop loading. Bring it with a manifest key or a new stage, or add \
             it to {path} in a pull request that says why no pack can hold it (see this file's top).",
            kind.name()
        );
        let gone: Vec<_> = listed.difference(&have).collect();
        assert!(
            gone.is_empty(),
            "the {} stage's 0.2 prelude no longer declares {gone:?}, which packs may use: {path}",
            kind.name()
        );
    }
}

/// The names on `tests/names-0.2/<list>.txt`.
fn listed(list: &str) -> BTreeSet<String> {
    let path = format!("{}/tests/names-0.2/{list}.txt", env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let lines = text.lines().map(str::trim);
    (lines.filter(|l| !l.is_empty() && !l.starts_with('#')))
        .map(str::to_owned)
        .collect()
}

/// The compute stages (`[[post]] compute = true`, `[[before_world]]`) came after the others and
/// start lists of their own.
#[test]
fn a_compute_prelude_declares_only_the_names_on_its_list() {
    let manifest = "schema = 1\nid = \"p\"\nname = \"P\"\nversion = \"1\"\napi = \"0.2\"\n\
                    [[before_world]]\nfile = \"before_world.wgsl\"\ndispatch = [64, 1, 1]\n\
                    workgroup = [64, 1, 1]\n[[post]]\nfile = \"compute.wgsl\"\ncompute = true\n\
                    dispatch = \"pixels\"\nworkgroup = [8, 8, 1]\n[[post]]\nfile = \"post.wgsl\"\n";
    let compute = "fn compute(input: ComputeInput) {}\n";
    let files = [
        (MANIFEST, manifest),
        ("before_world.wgsl", compute),
        ("compute.wgsl", compute),
        FILES[4],
    ];
    let pack = Pack::from_files(
        (files.iter())
            .map(|(n, t)| ((*n).to_owned(), t.as_bytes().to_vec()))
            .collect(),
    )
    .unwrap();
    check(&pack).unwrap();
    for kind in [StageKind::Compute, StageKind::BeforeWorld] {
        let file = format!("{}.wgsl", kind.name());
        let have = prelude_names(&compose(&pack, kind, &file), kind);
        assert_eq!(have, listed_names(kind).0, "{kind:?}");
    }
}

/// The rule at work: `[ray_tracing]` brings the names of `post_ray_tracing.txt` to the post
/// stage of the pack that set it and to its material stage where that calls `trace` or
/// `trace_any` (which has `block_pbr_maps` already), the same whether the renderer has the scene
/// or not.
#[test]
fn ray_tracing_brings_its_names_to_the_stages_of_the_pack_that_trace() {
    let material = "fn material(input: MaterialInput) -> Surface {\n    \
                    return Surface(input.albedo * f32(trace_any(input.position, input.normal, 8.0)));\n}\n";
    for (replaced, traces) in [(None, false), (Some(("material.wgsl", material)), true)] {
        let pack = pack_with("[ray_tracing]\n", replaced);
        for kind in StageKind::ALL {
            let file = format!("{}.wgsl", kind.name());
            let mut want = listed(kind.name());
            if kind == StageKind::Post || kind == StageKind::Material && traces {
                want.extend(listed("post_ray_tracing"));
            }
            if kind == StageKind::Post {
                want.insert("block_pbr_maps".to_owned());
            }
            for composed in [
                compose(&pack, kind, &file),
                compose_scene(&pack, kind, &file),
            ] {
                assert_eq!(prelude_names(&composed, kind), want, "{kind:?}");
            }
        }
    }
}

/// `[motion]` brings the post stage the motion guide's names, `post_motion.txt`, and no others.
#[test]
fn motion_brings_the_post_stage_only_the_motion_names() {
    let mut files = pack(None).files;
    let manifest = files.get_mut(MANIFEST).unwrap();
    manifest.extend_from_slice(b"[motion]\n");
    let pack = Pack::from_files(files).unwrap();
    for kind in StageKind::ALL {
        let composed = compose(&pack, kind, &format!("{}.wgsl", kind.name()));
        let have: BTreeSet<String> = declared(&composed.text[composed.parts[0].1.clone()])
            .into_iter()
            .filter(|n| !n.starts_with("horizon_"))
            .collect();
        let mut expected = listed(kind.name());
        if kind == horizon_shader_api::StageKind::Post {
            expected.extend(listed("post_motion"));
        }
        assert_eq!(have, expected, "{kind:?}");
    }
    check(&pack).unwrap();
}

/// `[[resource]]` brings the frame's epochs and the reasons' constants to every stage of the pack
/// that keeps resources, and to no other pack.
#[test]
fn kept_resources_bring_the_frame_s_epochs_to_their_pack_only() {
    let epochs = "    view_epoch: u32,\n    world_epoch: u32,\n    reset_reasons: u32,\n}";
    let resource =
        "[[resource]]\nname = \"cells\"\nbuffer = 64\nelement = \"u32\"\nkeep = \"view\"\n";
    let reasons = [
        "REASON_LOAD",
        "REASON_RESET",
        "REASON_WORLD",
        "REASON_TELEPORT",
        "REASON_PAUSE",
        "REASON_SIZE",
        "REASON_DISPLAY",
        "REASON_OPTION",
    ];
    let kept = pack_with(resource, None);
    for kind in StageKind::ALL {
        let file = format!("{}.wgsl", kind.name());
        let composed = compose(&kept, kind, &file);
        assert!(composed.text.contains(epochs), "{kind:?}");
        let (mut want, _) = listed_names(kind);
        want.extend(reasons.map(str::to_owned));
        assert_eq!(prelude_names(&composed, kind), want, "{kind:?}");
        assert!(
            !compose(&pack(None), kind, &file)
                .text
                .contains("view_epoch")
        );
    }
    let post = "fn post(input: PostInput) -> vec4<f32> {\n    \
                return input.color * f32((frame.reset_reasons & REASON_TELEPORT) == 0u);\n}\n";
    check(&pack_with(resource, Some(("post.wgsl", post)))).unwrap();
}

/// The rule for the entity layer: a pack brings `RAY_ENTITY` by naming it, and it comes to the
/// stages that trace (here the post stage alone) and to no other, with the scene or without it.
#[test]
fn ray_entity_comes_to_the_traced_stages_of_the_pack_that_names_it() {
    let post = "fn post(input: PostInput) -> vec4<f32> {\n    \
                let mob = trace_any_layers(vec3<f32>(0.0), vec3<f32>(0.0, 1.0, 0.0), 8.0, RAY_ENTITY);\n    \
                return input.color * f32(mob);\n}\n";
    let pack = pack_with("[ray_tracing]\n", Some(("post.wgsl", post)));
    check(&pack).unwrap();
    for kind in StageKind::ALL {
        let file = format!("{}.wgsl", kind.name());
        let (mut want, _) = listed_names(kind);
        if kind == StageKind::Post {
            let rays = [
                "RayHit",
                "trace",
                "trace_any",
                "trace_layers",
                "trace_any_layers",
                "RAY_SOLID",
                "RAY_TRANSLUCENT",
                "RAY_ENTITY",
                "hit_normal_map",
                "hit_specular_map",
                "block_pbr_maps",
            ];
            want.extend(rays.map(str::to_owned));
        }
        for composed in [
            compose(&pack, kind, &file),
            compose_scene(&pack, kind, &file),
        ] {
            assert_eq!(prelude_names(&composed, kind), want, "{kind:?}");
        }
    }
}

fn messages(result: Result<horizon_shader_api::Report, Vec<Problem>>) -> Vec<String> {
    result
        .expect_err("should fail")
        .iter()
        .map(Problem::to_string)
        .collect()
}

/// Why the rule is needed: a pack's own function by a name its stage's prelude has is refused,
/// so a name added to that prelude later would refuse a pack that passed before.
#[test]
fn a_pack_s_own_name_collides_only_with_its_own_stage_s_prelude() {
    let own = "fn depth_at(uv: vec2<f32>) -> f32 { return uv.x; }\n";
    let post = format!("{own}fn post(input: PostInput) -> vec4<f32> {{ return input.color; }}\n");
    let problems = messages(check(&pack(Some(("post.wgsl", &post)))));
    assert!(
        problems
            .iter()
            .any(|p| p.starts_with("post.wgsl:1:") && p.contains("redefinition of `depth_at`")),
        "{problems:#?}"
    );
    // The sky prelude has no `depth_at`: there the name is the pack's.
    let sky = format!("{own}fn sky(input: SkyInput) -> vec4<f32> {{ return input.color; }}\n");
    check(&pack(Some(("sky.wgsl", &sky)))).unwrap();
    // No prelude has `to_bt709` for a pack that doesn't ask for it: the helpers `[frame]
    // primaries = "bt2020"` will bring reach only the packs that set the key.
    let post = "fn to_bt709(c: vec3<f32>) -> vec3<f32> { return c; }\n\
                fn post(input: PostInput) -> vec4<f32> { return vec4<f32>(to_bt709(input.color.rgb), 1.0); }\n";
    check(&pack(Some(("post.wgsl", post)))).unwrap();
    // A stage added later brings its names to the packs that define it, and to no other stage:
    // `EntityInput` is still a post stage's own to define.
    let post = "struct EntityInput { color: vec4<f32> }\n\
                fn post(input: PostInput) -> vec4<f32> { return EntityInput(input.color).color; }\n";
    check(&pack(Some(("post.wgsl", post)))).unwrap();
}
