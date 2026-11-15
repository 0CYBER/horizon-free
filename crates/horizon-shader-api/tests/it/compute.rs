//! Shader API 0.2's compute passes (#16011 item 4): every manifest rule of `[[before_world]]`,
//! `[[post]] compute = true` and declared access, the WGSL composed around a compute file and
//! the resources each pass lists, and what the check refuses. The resources' own rules:
//! `tests/kept.rs`.
use horizon_shader_api::kept::{Dispatch, Element, Format, Keep, Shape};
use horizon_shader_api::{MANIFEST, Manifest, Pack, Problem, StageKind, check, compose};

const HEAD: &str = "schema = 1\nid = \"p\"\nname = \"P\"\nversion = \"1\"\napi = \"0.2\"\n";
/// The pass that draws the picture, after every other entry.
const LAST: &str = "[[post]]\nfile = \"last.wgsl\"\n";
/// A fixed texture and a buffer.
const RES: &str = "[[resource]]\nname = \"tex\"\nformat = \"r32float\"\nsize = [4, 4]\nkeep = \"view\"\n\
                   [[resource]]\nname = \"buf\"\nbuffer = 64\nelement = \"u32\"\nkeep = \"view\"\n";

fn parse(extra: &str) -> Result<Manifest, String> {
    Manifest::parse(&format!("{HEAD}{extra}{LAST}"))
}

fn refused(extra: &str, needle: &str) {
    let e = parse(extra).expect_err(extra);
    assert!(e.contains(needle), "{needle:?} not in {e:?}");
}

/// A compute `[[post]]` entry with `keys`.
fn compute(keys: &str) -> String {
    format!(
        "[[post]]\nfile = \"c.wgsl\"\ncompute = true\ndispatch = [4, 4, 1]\nworkgroup = [4, 4, 1]\n{keys}\n"
    )
}

fn resource(name: &str, shape: &str) -> String {
    format!("[[resource]]\nname = \"{name}\"\n{shape}\nkeep = \"view\"\n")
}

/// The design's example: a world cache, a 1×1 exposure and two histories.
const EXAMPLE: &str = r#"
[kept]
reach = 256

[[resource]]
name = "cache"
buffer = 16777216
element = "vec4<u32>"
keep = "world"

[[resource]]
name = "exposure"
format = "r32float"
size = [1, 1]
keep = "view"

[[resource]]
name = "ray_length"
format = "r32float"
scale = 1
keep = "view"

[[resource]]
name = "radiance"
format = "rgba16float"
scale = 1
keep = "view"

[[before_world]]
file = "cache_update.wgsl"
dispatch = [1048576, 1, 1]
workgroup = [64, 1, 1]
writes = ["cache"]

[[post]]
file = "ray_length.wgsl"
compute = true
dispatch = "pixels"
workgroup = [8, 8, 1]
writes = ["ray_length"]

[[post]]
file = "exposure.wgsl"
compute = true
dispatch = [16, 16, 1]
workgroup = [16, 16, 1]
writes = ["exposure"]

[[post]]
file = "denoise.wgsl"
output = "radiance"
reads = ["cache", "ray_length", "ray_length_previous", "radiance_previous", "exposure_previous"]

[[post]]
file = "light.wgsl"
reads = ["radiance"]

[[options]]
id = "bounces"
name = "Bounces"
type = "int"
default = 1
min = 1
max = 4
resets = "world"
"#;

const CACHE_UPDATE: &str = "\
override bounces: i32 = 1;
fn compute(input: ComputeInput) {
    if input.in_bounds {
        cache[input.cell.x] = cache[input.cell.x] + vec4<u32>(u32(bounces));
    }
}
";

const RAY_LENGTH: &str = "\
fn compute(input: ComputeInput) {
    if input.in_bounds {
        let uv = (vec2<f32>(input.cell.xy) + 0.5) / vec2<f32>(input.size.xy);
        let p = view_position(uv, depth_at(uv));
        textureStore(ray_length, vec2<i32>(input.cell.xy), vec4<f32>(length(p)));
    }
}
";

/// A reduction in workgroup memory: every invocation reaches every barrier.
const EXPOSURE: &str = "\
var<workgroup> sums: array<f32, 256>;
fn compute(input: ComputeInput) {
    var v = 0.0;
    if input.in_bounds {
        let uv = (vec2<f32>(input.cell.xy) + 0.5) / vec2<f32>(input.size.xy);
        v = dot(textureSampleLevel(previous, linear_sampler, uv, 0.0).rgb, frame.luminance);
    }
    sums[input.local] = v;
    for (var stride = 128u; stride > 0u; stride = stride / 2u) {
        workgroupBarrier();
        if input.local < stride {
            sums[input.local] += sums[input.local + stride];
        }
    }
    if input.local == 0u {
        textureStore(exposure, vec2<i32>(0), vec4<f32>(sums[0] / 256.0));
    }
}
";

const DENOISE: &str = "\
fn post(input: PostInput) -> vec4<f32> {
    let t = vec2<i32>(input.pixel);
    let moved = textureLoad(ray_length, t, 0).r - textureLoad(ray_length_previous, t, 0).r;
    let light = f32(cache[0].x) * textureLoad(exposure_previous, vec2<i32>(0), 0).r;
    return mix(input.color, textureLoad(radiance_previous, t, 0), 0.9) + vec4<f32>(moved + light);
}
";

const LIGHT: &str = "\
fn post(input: PostInput) -> vec4<f32> {
    return textureLoad(radiance, vec2<i32>(input.pixel), 0);
}
";

fn example_files() -> Vec<(&'static str, &'static str)> {
    vec![
        ("cache_update.wgsl", CACHE_UPDATE),
        ("ray_length.wgsl", RAY_LENGTH),
        ("exposure.wgsl", EXPOSURE),
        ("denoise.wgsl", DENOISE),
        ("light.wgsl", LIGHT),
    ]
}

/// A pack of `manifest` (after `HEAD`) and `files`.
fn pack(manifest: &str, files: &[(&str, &str)]) -> Pack {
    let mut all: std::collections::BTreeMap<String, Vec<u8>> = files
        .iter()
        .map(|(n, t)| ((*n).to_owned(), t.as_bytes().to_vec()))
        .collect();
    all.insert(MANIFEST.into(), format!("{HEAD}{manifest}").into_bytes());
    Pack::from_files(all).unwrap()
}

/// The example with `file` replaced by `text`.
fn example_with(file: &str, text: &str) -> Pack {
    let mut files = example_files();
    files.retain(|(n, _)| *n != file);
    files.push((file, text));
    pack(EXAMPLE, &files)
}

fn problems(pack: &Pack) -> Vec<String> {
    check(pack)
        .expect_err("should fail")
        .iter()
        .map(Problem::to_string)
        .collect()
}

#[test]
fn the_design_s_example_reads_as_declared() {
    let m = Manifest::parse(&format!("{HEAD}{EXAMPLE}")).unwrap();
    let k = &m.kept;
    assert_eq!(k.reach, Some(256));
    let shapes: Vec<_> = k
        .resources
        .iter()
        .map(|r| (r.name.as_str(), r.shape, r.keep))
        .collect();
    assert_eq!(
        shapes,
        [
            (
                "cache",
                Shape::Buffer {
                    bytes: 16 << 20,
                    element: Element::Vec4U32
                },
                Keep::World
            ),
            (
                "exposure",
                Shape::Fixed {
                    format: Format::R32Float,
                    size: [1, 1]
                },
                Keep::View
            ),
            (
                "ray_length",
                Shape::Scaled {
                    format: Format::R32Float,
                    scale: 1.0
                },
                Keep::View
            ),
            (
                "radiance",
                Shape::Scaled {
                    format: Format::Rgba16Float,
                    scale: 1.0
                },
                Keep::View
            ),
        ]
    );
    assert_eq!(
        k.before_world[0].dispatch,
        Some(Dispatch::Cells([1 << 20, 1, 1]))
    );
    assert_eq!(m.post[0].dispatch, Some(Dispatch::Pixels));
    assert_eq!(k.resets, [("bounces".to_owned(), Keep::World)]);
    assert!(m.computes());
    // The kept output is no post target of its own: the two R-relative textures take all four.
    assert!(m.post_targets().is_empty());
    assert!(m.files().contains(&"cache_update.wgsl"));
    assert!(!parse("").unwrap().computes());
}

#[test]
fn the_design_s_example_passes_the_check() {
    let report = check(&pack(EXAMPLE, &example_files())).unwrap();
    let kinds: Vec<_> = report
        .stages
        .iter()
        .map(|s| (s.kind, s.file.as_str()))
        .collect();
    assert_eq!(
        kinds,
        [
            (StageKind::BeforeWorld, "cache_update.wgsl"),
            (StageKind::Compute, "ray_length.wgsl"),
            (StageKind::Compute, "exposure.wgsl"),
            (StageKind::Post, "denoise.wgsl"),
            (StageKind::Post, "light.wgsl"),
        ]
    );
    let exposure = &report.stages[2].module.entry_points[0];
    assert_eq!(
        (exposure.stage, exposure.workgroup_size),
        (naga::ShaderStage::Compute, [16, 16, 1])
    );
}

#[test]
fn each_pass_declares_what_it_lists() {
    let p = pack(EXAMPLE, &example_files());
    let prelude = |kind, file| {
        let c = compose(&p, kind, file);
        c.text[c.parts[0].1.clone()].to_owned()
    };
    let before = prelude(StageKind::BeforeWorld, "cache_update.wgsl");
    assert!(
        before.contains("@group(3) @binding(64) var<storage, read_write> cache: array<vec4<u32>>;")
    );
    assert!(!before.contains("ray_length") && !before.contains("scene_color"));
    let ray = prelude(StageKind::Compute, "ray_length.wgsl");
    assert!(
        ray.contains("@group(3) @binding(68) var ray_length: texture_storage_2d<r32float, write>;")
    );
    assert!(!ray.contains("cache") && !ray.contains("PostInput"));
    let denoise = prelude(StageKind::Post, "denoise.wgsl");
    for line in [
        "@group(3) @binding(64) var<storage, read> cache: array<vec4<u32>>;",
        "@group(3) @binding(67) var exposure_previous: texture_2d<f32>;",
        "@group(3) @binding(68) var ray_length: texture_2d<f32>;",
        "@group(3) @binding(69) var ray_length_previous: texture_2d<f32>;",
        "@group(3) @binding(71) var radiance_previous: texture_2d<f32>;",
    ] {
        assert!(denoise.contains(line), "{line}");
    }
    // Its output is its attachment, not a texture it reads.
    assert!(!denoise.contains("var radiance:"));
    let light = prelude(StageKind::Post, "light.wgsl");
    assert!(light.contains("@group(3) @binding(70) var radiance: texture_2d<f32>;"));
}

/// What a runtime builds its layouts from, `kept_bindings`, is what each pass's module declares:
/// the binding, the name, and whether it is written.
#[test]
fn kept_bindings_are_what_each_pass_declares() {
    use horizon_shader_api::stages::{KEPT_BINDING, KeptAccess, entry, kept_bindings};
    let p = pack(EXAMPLE, &example_files());
    let m = &p.manifest;
    for stage in check(&p).unwrap().stages {
        let listed = entry(m, stage.kind, &stage.file, None).unwrap();
        let want: Vec<_> = (kept_bindings(m, listed).iter())
            .map(|b| {
                let name = &m.kept.resources[b.resource].name;
                match b.access {
                    KeptAccess::Previous => (b.binding, format!("{name}_previous"), false),
                    a => (b.binding, name.clone(), a == KeptAccess::Write),
                }
            })
            .collect();
        let module = &stage.module;
        let have: Vec<_> = (module.global_variables.iter())
            .filter_map(|(_, g)| {
                let b = g.binding.as_ref().filter(|b| b.binding >= KEPT_BINDING)?;
                let written = match (g.space, &module.types[g.ty].inner) {
                    (naga::AddressSpace::Storage { access }, _) => {
                        access.contains(naga::StorageAccess::STORE)
                    }
                    (_, naga::TypeInner::Image { class, .. }) => {
                        matches!(class, naga::ImageClass::Storage { .. })
                    }
                    _ => false,
                };
                Some((b.binding, g.name.clone()?, written))
            })
            .collect();
        assert!(!have.is_empty(), "{}", stage.file);
        assert_eq!(have, want, "{}", stage.file);
    }
}

#[test]
fn the_entry_runs_every_invocation_with_in_bounds() {
    let p = pack(EXAMPLE, &example_files());
    let entry = |kind, file| {
        compose(&p, kind, file)
            .parts
            .last()
            .map(|(_, r)| r.clone())
            .unwrap()
    };
    let c = compose(&p, StageKind::Compute, "exposure.wgsl");
    let text = &c.text[entry(StageKind::Compute, "exposure.wgsl")];
    assert!(
        text.contains("@compute @workgroup_size(16, 16, 1)"),
        "{text}"
    );
    assert!(
        text.contains("let size = vec3<u32>(16u, 16u, 1u);"),
        "{text}"
    );
    assert!(text.contains("compute(ComputeInput(cell, local, size, all(cell < size)));"));
    let c = compose(&p, StageKind::Compute, "ray_length.wgsl");
    let text = &c.text[entry(StageKind::Compute, "ray_length.wgsl")];
    assert!(
        text.contains("vec3<u32>(vec2<u32>(frame.resolution), 1u)"),
        "{text}"
    );
}

#[test]
fn an_atomic_buffer_is_atomic_only_where_written() {
    let manifest = format!(
        "{}[[before_world]]\nfile = \"count.wgsl\"\ndispatch = [64, 1, 1]\nworkgroup = [64, 1, 1]\n\
         writes = [\"counts\"]\n[[post]]\nfile = \"show.wgsl\"\nreads = [\"counts\"]\n",
        resource("counts", "buffer = 256\nelement = \"atomic<u32>\"")
    );
    let count = "fn compute(input: ComputeInput) {\n    if input.in_bounds {\n        \
                 atomicAdd(&counts[input.cell.x], 1u);\n    }\n}\n";
    let show = "fn post(input: PostInput) -> vec4<f32> { return input.color * f32(counts[0]); }\n";
    let p = pack(&manifest, &[("count.wgsl", count), ("show.wgsl", show)]);
    check(&p).unwrap();
    let before = compose(&p, StageKind::BeforeWorld, "count.wgsl").text;
    assert!(before.contains("var<storage, read_write> counts: array<atomic<u32>>;"));
    let post = compose(&p, StageKind::Post, "show.wgsl").text;
    assert!(post.contains("var<storage, read> counts: array<u32>;"));
}

#[test]
fn a_file_naming_a_resource_it_did_not_list_is_refused() {
    let light = "fn post(input: PostInput) -> vec4<f32> {\n    \
                 return textureLoad(radiance, vec2<i32>(input.pixel), 0) * f32(cache[0].x);\n}\n";
    let p = problems(&example_with("light.wgsl", light));
    assert!(
        p.iter().any(|p| p.starts_with("light.wgsl:2:")
            && p.contains("`cache` is a kept resource this pass doesn't list in `reads`/`writes`")),
        "{p:#?}"
    );
    let update = "fn compute(input: ComputeInput) {\n    \
                  if input.in_bounds { cache[0].x = u32(textureLoad(exposure, vec2<i32>(0), 0).x); }\n}\n";
    let p = problems(&example_with("cache_update.wgsl", update));
    assert!(
        p.iter().any(|p| p.starts_with("cache_update.wgsl:2:")
            && p.contains("`exposure` is a kept resource this pass doesn't list")),
        "{p:#?}"
    );
    // Only what a pass lists is declared: a name of its own, a local or a member by a resource's
    // name, is its own.
    let ray = "struct Cell { cache: u32 }\n\
               fn helper() -> u32 { let cache = 1u; return Cell(cache).cache; }\n\
               fn compute(input: ComputeInput) {\n    \
               let radiance_previous = f32(helper());\n    \
               if input.in_bounds {\n        \
               textureStore(ray_length, vec2<i32>(input.cell.xy), vec4<f32>(radiance_previous));\n    \
               }\n}\n";
    check(&example_with("ray_length.wgsl", ray)).unwrap();
}

/// Two entries may share a file and differ in the rest: each is built as its own entry says.
#[test]
fn a_file_reused_by_two_entries_is_built_for_each() {
    let manifest = format!(
        "{}{}[[post]]\nfile = \"c.wgsl\"\ncompute = true\ndispatch = [64, 1, 1]\n\
         workgroup = [64, 1, 1]\nwrites = [\"a\"]\n\
         [[post]]\nfile = \"c.wgsl\"\ncompute = true\ndispatch = \"pixels\"\n\
         workgroup = [8, 8, 1]\nwrites = [\"b\"]\n{LAST}",
        resource("a", "buffer = 256\nelement = \"u32\""),
        resource("b", "buffer = 256\nelement = \"u32\""),
    );
    // Each entry's own access list: `a` resolves in the first, `b` in the second.
    let c = "fn compute(input: ComputeInput) { if input.in_bounds { a[0] = input.size.x; } }\n";
    let post = "fn post(input: PostInput) -> vec4<f32> { return input.color; }\n";
    let p = problems(&pack(&manifest, &[("c.wgsl", c), ("last.wgsl", post)]));
    assert!(
        p.iter()
            .any(|p| p.contains("`a` is a kept resource this pass doesn't list")),
        "{p:#?}"
    );
    let c = "fn compute(input: ComputeInput) {}\n";
    let report = check(&pack(&manifest, &[("c.wgsl", c), ("last.wgsl", post)])).unwrap();
    let sizes: Vec<_> = (report.stages.iter().take(2))
        .map(|s| s.module.entry_points[0].workgroup_size)
        .collect();
    assert_eq!(sizes, [[64, 1, 1], [8, 8, 1]]);
}

/// In a pack with kept resources, ordinary `[[post]]` entries sharing a file are built per
/// entry too: each declares only what its own `reads` list.
#[test]
fn ordinary_entries_sharing_a_file_read_what_each_lists() {
    let post = "fn post(input: PostInput) -> vec4<f32> { return input.color * f32(cache[0]); }\n";
    let last = "fn post(input: PostInput) -> vec4<f32> { return input.color; }\n";
    let built = |first: &str, second: &str| {
        let manifest = format!(
            "{}[[post]]\nfile = \"p.wgsl\"\n{first}\n[[post]]\nfile = \"p.wgsl\"\n{second}\n{LAST}",
            resource("cache", "buffer = 64\nelement = \"u32\""),
        );
        check(&pack(&manifest, &[("p.wgsl", post), ("last.wgsl", last)]))
    };
    let reads = "reads = [\"cache\"]";
    built(reads, reads).unwrap();
    for (first, second) in [(reads, ""), ("", reads)] {
        let p: Vec<_> = built(first, second).expect_err("one entry doesn't list `cache`");
        let p: Vec<_> = (p.iter().map(Problem::to_string))
            .filter(|p| p.contains("`cache` is a kept resource this pass doesn't list"))
            .collect();
        assert_eq!(p.len(), 1, "{first:?} then {second:?}: {p:#?}");
    }
}

/// The 32-bit formats are read with `textureLoad` only, also through a helper's argument;
/// `rgba16float` may be sampled.
#[test]
fn a_32_bit_kept_texture_is_not_sampled() {
    let sampled = [
        "textureSampleLevel(ray_length_previous, linear_sampler, input.uv, 0.0)",
        "textureSampleLevel(ray_length, nearest_sampler, input.uv, 0.0)",
        "textureGather(0, ray_length, linear_sampler, input.uv)",
        "blurred(ray_length_previous, input.uv)",
    ];
    for read in sampled {
        let denoise = format!(
            "fn blurred(t: texture_2d<f32>, uv: vec2<f32>) -> vec4<f32> {{\n    \
             return textureSampleLevel(t, linear_sampler, uv, 0.0);\n}}\n\
             fn post(input: PostInput) -> vec4<f32> {{\n    \
             return {read} + f32(cache[0].x) + textureLoad(exposure_previous, vec2<i32>(0), 0);\n}}\n"
        );
        let p = problems(&example_with("denoise.wgsl", &denoise));
        assert!(
            p.iter().any(|p| p.starts_with("denoise.wgsl:")
                && p.contains("is a 32-bit kept texture: read it with `textureLoad`")),
            "{read}: {p:#?}"
        );
    }
    let light = "fn post(input: PostInput) -> vec4<f32> {\n    \
                 return textureSampleLevel(radiance, linear_sampler, input.uv, 0.0);\n}\n";
    check(&example_with("light.wgsl", light)).unwrap();
}

#[test]
fn a_compute_file_keeps_the_api_s_rules() {
    let p = problems(&example_with(
        "exposure.wgsl",
        "fn compute(input: ComputeInput) -> f32 { return 1.0; }\n",
    ));
    assert!(
        p.iter().any(|p| p.contains(
            "`fn compute(ComputeInput) -> f32` should be `fn compute(input: ComputeInput)`"
        )),
        "{p:#?}"
    );
    let built = "fn compute(input: ComputeInput) { let i = ComputeInput(vec3<u32>(0u), 0u, vec3<u32>(1u), true); }\n";
    let p = problems(&example_with("exposure.wgsl", built));
    assert!(
        p.iter().any(|p| p.contains("constructs `ComputeInput`")),
        "{p:#?}"
    );
    let entry = "@compute @workgroup_size(1) fn mine() {}\nfn compute(input: ComputeInput) {}\n";
    let p = problems(&example_with("exposure.wgsl", entry));
    assert!(
        p.iter()
            .any(|p| p.contains("declares the entry point `mine`")),
        "{p:#?}"
    );
}

#[test]
fn pass_counts_are_bounded() {
    let before = |i: usize| {
        format!(
            "[[before_world]]\nfile = \"b{i}.wgsl\"\ndispatch = [1, 1, 1]\nworkgroup = [1, 1, 1]\n"
        )
    };
    parse(&(0..4).map(before).collect::<String>()).unwrap();
    refused(
        &(0..5).map(before).collect::<String>(),
        "5 [[before_world]] entries; a pack may have 4",
    );
    let computes = |n: usize| (0..n).map(|_| compute("")).collect::<String>();
    parse(&computes(8)).unwrap();
    refused(
        &computes(9),
        "9 compute [[post]] entries; a pack may have 8",
    );
    // Compute entries don't count against the 12 post passes.
    let passes = (0..12)
        .map(|i| format!("[[post]]\nfile = \"p{i}.wgsl\"\n"))
        .collect::<String>();
    Manifest::parse(&format!("{HEAD}{}{passes}", computes(8))).unwrap();
}

#[test]
fn a_compute_entry_is_a_dispatch_in_the_chain() {
    refused(
        &compute("output = \"glow\""),
        "a compute pass has no `output` or `scale`",
    );
    refused(
        &compute("scale = 0.5"),
        "a compute pass has no `output` or `scale`",
    );
    let last = Manifest::parse(&format!("{HEAD}{}", compute(""))).unwrap_err();
    assert!(last.contains("can't be the last [[post]] entry"), "{last}");
    refused(
        "[[post]]\nfile = \"c.wgsl\"\ncompute = true\ndispatch = \"pixels\"\n",
        "needs `dispatch` and `workgroup`",
    );
    refused(
        "[[post]]\nfile = \"c.wgsl\"\ncompute = true\nworkgroup = [1, 1, 1]\n",
        "needs `dispatch` and `workgroup`",
    );
    let shaped = |dispatch: &str, workgroup: &str| {
        format!(
            "[[post]]\nfile = \"c.wgsl\"\ncompute = true\ndispatch = {dispatch}\nworkgroup = {workgroup}\n"
        )
    };
    refused(
        &shaped("\"pixels\"", "[512, 1, 1]"),
        "workgroup [512, 1, 1]",
    );
    refused(
        &shaped("\"pixels\"", "[16, 16, 2]"),
        "at most 256 invocations",
    );
    refused(&shaped("\"pixels\"", "[0, 1, 1]"), "workgroup [0, 1, 1]");
    refused(&shaped("[0, 1, 1]", "[1, 1, 1]"), "dispatch [0, 1, 1]");
    refused(
        &shaped("[70000, 1, 1]", "[1, 1, 1]"),
        "at most 65535 workgroups",
    );
    parse(&shaped("[70000, 1, 1]", "[2, 1, 1]")).unwrap();
    refused(
        &shaped("\"rows\"", "[1, 1, 1]"),
        "dispatch is \"pixels\" or [x, y, z] cells",
    );
    // An ordinary pass writes its output only.
    for key in [
        "dispatch = \"pixels\"",
        "workgroup = [1, 1, 1]",
        "writes = [\"buf\"]",
    ] {
        refused(
            &format!("{RES}[[post]]\nfile = \"a.wgsl\"\n{key}\n"),
            "are a compute pass's",
        );
    }
    refused(
        "[[before_world]]\nfile = \"b.wgsl\"\ncompute = true\ndispatch = [1, 1, 1]\nworkgroup = [1, 1, 1]\n",
        "no `compute` key",
    );
    refused(
        "[[before_world]]\nfile = \"b.wgsl\"\noutput = \"x\"\ndispatch = [1, 1, 1]\nworkgroup = [1, 1, 1]\n",
        "`scale` and `output` are keys of [[post]] only",
    );
    for key in [
        "compute = true",
        "reads = [\"tex\"]",
        "dispatch = \"pixels\"",
    ] {
        refused(
            &format!("{RES}[material]\nfile = \"m.wgsl\"\n{key}\n"),
            "are keys of [[post]] and [[before_world]]",
        );
    }
}

#[test]
fn access_names_declared_resources_with_one_writer() {
    parse(&format!(
        "{RES}{}",
        compute("reads = [\"tex_previous\", \"buf\"]\nwrites = [\"tex\", \"buf\"]")
    ))
    .unwrap();
    let cases = [
        (
            "reads = [\"nothing\"]",
            "reads \"nothing\", no [[resource]]",
        ),
        (
            "reads = [\"buf_previous\"]",
            "reads \"buf_previous\", no [[resource]] or texture's _previous",
        ),
        ("reads = [\"tex\", \"tex\"]", "reads \"tex\" twice"),
        (
            "writes = [\"tex_previous\"]",
            "writes \"tex_previous\", no [[resource]]",
        ),
        (
            "writes = [\"buf\", \"buf\"]",
            "writes \"buf\" twice: one writer a pass",
        ),
        (
            "reads = [\"tex\"]\nwrites = [\"tex\"]",
            "writes \"tex\" and reads it back",
        ),
    ];
    for (keys, needle) in cases {
        refused(&format!("{RES}{}", compute(keys)), needle);
    }
    let output = |keys: &str| format!("{RES}[[post]]\nfile = \"a.wgsl\"\n{keys}\n");
    parse(&output(
        "output = \"tex\"\nreads = [\"tex_previous\", \"buf\"]",
    ))
    .unwrap();
    refused(
        &output("output = \"tex\"\nreads = [\"tex\"]"),
        "\"tex\" is its output",
    );
    refused(&output("output = \"tex\"\nscale = 0.5"), "with no `scale`");
    refused(&output("output = \"buf\""), "is a kept buffer");
    refused(
        &output("output = \"tex_previous\""),
        "is last frame's: nothing writes it",
    );
    let last = Manifest::parse(&format!(
        "{HEAD}{RES}[[post]]\nfile = \"z.wgsl\"\noutput = \"tex\"\n"
    ));
    assert!(
        last.unwrap_err()
            .contains("the last [[post]] pass draws the picture")
    );
}

/// #16011 item 4's caps on compute work: 2^25 invocations a dispatch and 2^26 a frame over
/// every compute pass, counted as whole workgroups, a `"pixels"` dispatch as 3840×2160 rounded
/// up to them.
#[test]
fn compute_work_is_capped_per_dispatch_and_per_frame() {
    use horizon_shader_api::kept::invocations;
    let entry = |dispatch: &str, workgroup: &str| {
        format!(
            "[[post]]\nfile = \"c.wgsl\"\ncompute = true\ndispatch = {dispatch}\n\
             workgroup = {workgroup}\n"
        )
    };
    let counted = |dispatch: &str, workgroup: &str| {
        let m = parse(&entry(dispatch, workgroup)).unwrap();
        invocations(&m.post[0])
    };
    // 3840×2160 in 8×8 workgroups fills them; in 7×7 the edges round up to 549×309 of them.
    assert_eq!(counted("\"pixels\"", "[8, 8, 1]"), 8_294_400);
    assert_eq!(counted("\"pixels\"", "[7, 7, 1]"), 549 * 309 * 49);
    // A partly filled workgroup counts in full.
    assert_eq!(counted("[100, 1, 1]", "[64, 1, 1]"), 128);
    // 2^25 a dispatch: 8192×4096 cells take it.
    parse(&entry("[8192, 4096, 1]", "[256, 1, 1]")).unwrap();
    refused(
        &entry("[8192, 4097, 1]", "[256, 1, 1]"),
        "c.wgsl: 33562624 invocations (whole workgroups, \"pixels\" counted at 3840×2160); a \
         dispatch may run 33554432",
    );
    refused(
        &entry("[8193, 4096, 1]", "[256, 1, 1]"),
        "c.wgsl: 34603008 invocations",
    );
    // 2^26 a frame: seven pixel dispatches and 9,048,064 cells take it, a workgroup more doesn't.
    let pixels = entry("\"pixels\"", "[8, 8, 1]").repeat(7);
    let with = |cells: u32| {
        format!(
            "{pixels}{}",
            entry(&format!("[{cells}, 1, 1]"), "[256, 1, 1]")
        )
    };
    parse(&with(9_048_064)).unwrap();
    refused(
        &with(9_048_065),
        "the compute passes run 67109120 invocations a frame (whole workgroups, \"pixels\" \
         counted at 3840×2160); a pack may run 67108864",
    );
}

/// An `rgba32uint` kept texture: written by a compute pass as storage, read as
/// `texture_2d<u32>` with `textureLoad`, `_previous` too.
#[test]
fn an_rgba32uint_texture_is_written_by_compute_and_loaded_as_u32() {
    let manifest = format!(
        "{}[[post]]\nfile = \"c.wgsl\"\ncompute = true\ndispatch = [4, 4, 1]\n\
         workgroup = [4, 4, 1]\nwrites = [\"ints\"]\n\
         [[post]]\nfile = \"last.wgsl\"\nreads = [\"ints\", \"ints_previous\"]\n",
        resource("ints", "format = \"rgba32uint\"\nsize = [4, 4]")
    );
    let write = "fn compute(input: ComputeInput) {\n    \
                 if input.in_bounds { textureStore(ints, input.cell.xy, vec4<u32>(input.cell.x)); }\n}\n";
    let read = "fn post(input: PostInput) -> vec4<f32> {\n    \
                let p = vec2<i32>(input.pixel);\n    \
                let n = textureLoad(ints, p, 0).x + textureLoad(ints_previous, p, 0).x;\n    \
                return input.color * f32(n);\n}\n";
    let p = pack(&manifest, &[("c.wgsl", write), ("last.wgsl", read)]);
    check(&p).unwrap();
    let prelude = |kind, file| {
        let c = compose(&p, kind, file);
        c.text[c.parts[0].1.clone()].to_owned()
    };
    let written = "@group(3) @binding(64) var ints: texture_storage_2d<rgba32uint, write>;";
    assert!(prelude(StageKind::Compute, "c.wgsl").contains(written));
    let last = prelude(StageKind::Post, "last.wgsl");
    assert!(last.contains("@group(3) @binding(64) var ints: texture_2d<u32>;"));
    assert!(last.contains("@group(3) @binding(65) var ints_previous: texture_2d<u32>;"));
}

/// `[[before_world]]` dispatches are capped as compute `[[post]]` ones are, and count toward the
/// same 2^26 invocations a frame.
#[test]
fn before_world_work_shares_the_frame_cap() {
    let entry = |list: &str, dispatch: &str| {
        let compute = if list == "post" {
            "compute = true\n"
        } else {
            ""
        };
        format!(
            "[[{list}]]\nfile = \"c.wgsl\"\n{compute}dispatch = {dispatch}\nworkgroup = [8, 8, 1]\n"
        )
    };
    // 2^25 a dispatch holds for them too.
    refused(
        &entry("before_world", "[8192, 4104, 1]"),
        "c.wgsl: 33619968 invocations",
    );
    // One 2^25 dispatch in each list: exactly 2^26 a frame fits; a workgroup more in either
    // list alone fits too, but not with the other list.
    let full = "[8192, 4096, 1]";
    let before = entry("before_world", full);
    let post = format!("{}{}", entry("post", full), entry("post", "[8, 8, 1]"));
    parse(&before).unwrap();
    parse(&post).unwrap();
    parse(&format!("{before}{}", entry("post", full))).unwrap();
    refused(
        &format!("{before}{post}"),
        "the compute passes run 67108928 invocations a frame",
    );
}
