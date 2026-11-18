//! `[material] reads` (#16011 item 4, step 7): the terrain reads kept resources, this frame's
//! version of a texture a `[[before_world]]` pass wrote, any texture's `_previous` and a buffer,
//! declared in its module as a pass's are; it writes none, and what also draws the shadow reads
//! none.
use horizon_shader_api::stages::{KEPT_BINDING, KeptAccess, kept_bindings};
use horizon_shader_api::{MANIFEST, Manifest, Pack, StageKind, check, compose, compose_scene};

const HEAD: &str = "schema = 1\nid = \"p\"\nname = \"P\"\nversion = \"1\"\napi = \"0.2\"\n";
/// A fixed texture and a buffer, both written by a `[[before_world]]` pass.
const KEPT: &str = "[[resource]]\nname = \"tex\"\nformat = \"r32float\"\nsize = [4, 4]\nkeep = \"view\"\n\
                    [[resource]]\nname = \"buf\"\nbuffer = 64\nelement = \"u32\"\nkeep = \"view\"\n";
const WORLD: &str = "[[before_world]]\nfile = \"w.wgsl\"\ndispatch = [4, 4, 1]\nworkgroup = [4, 4, 1]\n\
                     writes = [\"tex\", \"buf\"]\n";
const WRITE: &str = "fn compute(input: ComputeInput) {\n    if input.in_bounds {\n        \
                     textureStore(tex, vec2<i32>(input.cell.xy), vec4<f32>(1.0));\n        \
                     buf[input.cell.x] = 1u;\n    }\n}\n";
const READ: &str = "fn material(input: MaterialInput) -> Surface {\n    \
                    let now = textureLoad(tex, vec2<i32>(0), 0).x;\n    \
                    let then = textureLoad(tex_previous, vec2<i32>(0), 0).x;\n    \
                    return Surface(input.albedo * (now + then + f32(buf[0])));\n}\n";
const PLAIN: &str =
    "fn material(input: MaterialInput) -> Surface { return Surface(input.albedo); }\n";
const ALL: &str = "reads = [\"tex\", \"tex_previous\", \"buf\"]\n";

fn material(reads: &str) -> String {
    format!("{KEPT}{WORLD}[material]\nfile = \"m.wgsl\"\n{reads}")
}

fn parse(extra: &str) -> Result<Manifest, String> {
    Manifest::parse(&format!("{HEAD}{extra}"))
}

fn refused(extra: &str, needle: &str) {
    let e = parse(extra).expect_err(extra);
    assert!(e.contains(needle), "{needle:?} not in {e:?}");
}

fn pack(manifest: &str, files: &[(&str, &str)]) -> Pack {
    let mut all: std::collections::BTreeMap<String, Vec<u8>> = files
        .iter()
        .map(|(n, t)| ((*n).to_owned(), t.as_bytes().to_vec()))
        .collect();
    all.insert(MANIFEST.into(), format!("{HEAD}{manifest}").into_bytes());
    Pack::from_files(all).unwrap()
}

fn problems(manifest: &str, files: &[(&str, &str)]) -> Vec<String> {
    let e = check(&pack(manifest, files)).expect_err("should fail");
    e.iter().map(ToString::to_string).collect()
}

#[test]
fn the_material_reads_what_it_lists() {
    let m = parse(&material(ALL)).unwrap();
    assert_eq!(m.material.unwrap().reads, ["tex", "tex_previous", "buf"]);
    // Last frame's texture and a buffer need no writer before the terrain.
    let late = format!(
        "{KEPT}[material]\nfile = \"m.wgsl\"\nreads = [\"tex_previous\", \"buf\"]\n\
         [[post]]\nfile = \"c.wgsl\"\ncompute = true\ndispatch = [4, 4, 1]\nworkgroup = [4, 4, 1]\n\
         writes = [\"tex\", \"buf\"]\n[[post]]\nfile = \"last.wgsl\"\n"
    );
    parse(&late).unwrap();
    // This frame's texture needs a `[[before_world]]` writer: a post pass writes it after the
    // terrain draws.
    let now = late.replace("\"tex_previous\"", "\"tex\"");
    refused(
        &now,
        "m.wgsl: reads \"tex\", this frame's, which no [[before_world]] pass writes before the \
         terrain draws: read tex_previous",
    );
    let output = format!(
        "{KEPT}[material]\nfile = \"m.wgsl\"\nreads = [\"tex\"]\n[[post]]\nfile = \"a.wgsl\"\n\
         output = \"tex\"\n[[post]]\nfile = \"last.wgsl\"\n"
    );
    refused(&output, "read tex_previous");
    let only_buf = WORLD.replace("\"tex\", ", "");
    refused(
        &format!("{KEPT}{only_buf}[material]\nfile = \"m.wgsl\"\nreads = [\"tex\"]\n"),
        "read tex_previous",
    );
}

#[test]
fn the_material_names_declared_resources_once_and_writes_none() {
    for (reads, needle) in [
        (
            "reads = [\"nothing\"]",
            "m.wgsl: reads \"nothing\", no [[resource]]",
        ),
        (
            "reads = [\"buf_previous\"]",
            "no [[resource]] or texture's _previous",
        ),
        ("reads = [\"buf\", \"buf\"]", "reads \"buf\" twice"),
    ] {
        refused(&material(reads), needle);
    }
    let keys = "and `reads` of [material], [entity] and [sky]: they draw with kept resources, \
                compute passes write them";
    for key in [
        "writes = [\"buf\"]",
        "compute = true",
        "dispatch = [1, 1, 1]",
        "workgroup = [1, 1, 1]",
    ] {
        refused(&material(&format!("{key}\n")), keys);
    }
    // The other stages read none (`[entity]` and `[sky]` do: entity_sky_reads.rs).
    for stage in ["vertex", "weather", "tonemap"] {
        refused(
            &format!(
                "{}[{stage}]\nfile = \"s.wgsl\"\nreads = [\"buf\"]\n",
                material("")
            ),
            keys,
        );
    }
}

/// The material's module declares what it lists at the pass's bindings, as `kept_bindings`
/// gives them: this frame's texture and `_previous` sampled, a buffer read-only.
#[test]
fn the_material_module_declares_its_reads() {
    let p = pack(&material(ALL), &[("w.wgsl", WRITE), ("m.wgsl", READ)]);
    let report = check(&p).unwrap();
    let stage = (report.stages.iter())
        .find(|s| s.kind == StageKind::Material)
        .unwrap();
    let listed = p.manifest.material.as_ref().unwrap();
    let want: Vec<_> = (kept_bindings(&p.manifest, listed).iter())
        .map(|b| (b.binding, b.access))
        .collect();
    assert_eq!(
        want,
        [
            (KEPT_BINDING, KeptAccess::Read),
            (KEPT_BINDING + 1, KeptAccess::Previous),
            (KEPT_BINDING + 2, KeptAccess::Read)
        ]
    );
    let text = compose(&p, StageKind::Material, "m.wgsl").text;
    for line in [
        "@group(3) @binding(64) var tex: texture_2d<f32>;",
        "@group(3) @binding(65) var tex_previous: texture_2d<f32>;",
        "@group(3) @binding(66) var<storage, read> buf: array<u32>;",
    ] {
        assert!(text.contains(line), "{line}");
    }
    let bound: Vec<_> = (stage.module.global_variables.iter())
        .filter_map(|(_, g)| g.binding.as_ref().filter(|b| b.binding >= KEPT_BINDING))
        .map(|b| b.binding)
        .collect();
    assert_eq!(bound, [64, 65, 66]);
}

/// A material file naming a resource its `reads` doesn't list is refused; a vertex file naming
/// one, or `displacement` reaching one, is refused too: both run in a vertex shader and draw the
/// shadow, which gets no kept resources.
#[test]
fn only_the_material_fragment_reads_kept_resources() {
    let only_buf = material("reads = [\"buf\"]\n");
    let p = problems(&only_buf, &[("w.wgsl", WRITE), ("m.wgsl", READ)]);
    assert!(
        p.iter().any(|p| p.starts_with("m.wgsl:2:")
            && p.contains("`tex` is a kept resource the material doesn't list in `reads`")),
        "{p:#?}"
    );
    let wave = "fn vertex(input: VertexInput) -> vec3<f32> {\n    \
                return input.position + vec3(0.0, f32(buf[0]), 0.0);\n}\n";
    let vertex = format!("{}[vertex]\nfile = \"v.wgsl\"\n", material(ALL));
    let p = problems(
        &vertex,
        &[("w.wgsl", WRITE), ("m.wgsl", READ), ("v.wgsl", wave)],
    );
    assert!(
        p.iter().any(|p| p.starts_with("v.wgsl:2:")
            && p.contains("`buf` is a kept resource, which the vertex stage doesn't read")),
        "{p:#?}"
    );
    let displaced = format!("{}[displacement]\n", material(ALL));
    let deep = format!(
        "{READ}fn depth() -> f32 {{ return f32(buf[0]); }}\n\
         fn displacement(input: DisplacementInput) -> f32 {{ return depth(); }}\n"
    );
    let p = problems(&displaced, &[("w.wgsl", WRITE), ("m.wgsl", &deep)]);
    assert!(
        p.iter().any(|p| p.contains(
            "`displacement` reaches a kept resource: it runs in a vertex shader, which reads \
             none, and casts the shadow, which gets none"
        )),
        "{p:#?}"
    );
    let flat = format!(
        "{READ}fn displacement(input: DisplacementInput) -> f32 {{ return input.height; }}\n"
    );
    check(&pack(&displaced, &[("w.wgsl", WRITE), ("m.wgsl", &flat)])).unwrap();
}

/// A material without `reads` in a kept, traced pack composes as before `[material] reads`:
/// nothing at the kept bindings, byte for byte the same with the scene and without it.
#[test]
fn a_material_without_reads_composes_as_before() {
    let manifest = format!("[ray_tracing]\n{}", material(""));
    let traced = "fn material(input: MaterialInput) -> Surface {\n    \
                  let hit = trace(input.position, vec3(0.0, 1.0, 0.0), 8.0);\n    \
                  return Surface(input.albedo * select(1.0, 0.5, hit.distance >= 0.0));\n}\n";
    let p = pack(&manifest, &[("w.wgsl", WRITE), ("m.wgsl", traced)]);
    check(&p).unwrap();
    let composed = [
        compose(&p, StageKind::Material, "m.wgsl"),
        compose_scene(&p, StageKind::Material, "m.wgsl"),
    ];
    for c in &composed {
        assert!(!c.text.contains("@binding(64)"));
    }
    let hashes: Vec<String> = (composed.iter())
        .map(|c| horizon_shader_api::package::sha256_hex(c.text.as_bytes()))
        .collect();
    assert_eq!(
        hashes,
        [
            "3603cefa770143294f4a0fe1a9d799cd0daa52c95fd11ef9dad28a93d1619458",
            "a8ba6e8a89b7865fb0e1b897330bbca5ab48605f55984f969aade62d5d890655",
        ]
    );
    let plain = pack(&material(""), &[("w.wgsl", WRITE), ("m.wgsl", PLAIN)]);
    check(&plain).unwrap();
}
