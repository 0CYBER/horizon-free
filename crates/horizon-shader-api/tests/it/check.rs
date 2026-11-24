//! `check`: the sample pack passes, and each kind of mistake is reported
//! with the pack's own file and line.
use horizon_shader_api::{MANIFEST, Pack, Problem, StageKind, check};
use std::path::Path;

fn sample() -> Pack {
    Pack::read(Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/samples/schema-1"
    )))
    .unwrap()
}

/// The sample with `file` replaced by `text`, checked.
fn problems(file: &str, text: &str) -> Vec<Problem> {
    let mut files = sample().files;
    files.insert(file.into(), text.as_bytes().to_vec());
    check(&Pack::from_files(files).unwrap()).expect_err("should fail")
}

fn one(file: &str, text: &str, needle: &str) -> Problem {
    let p = problems(file, text);
    assert!(
        p.iter().any(|p| p.message.contains(needle)),
        "{needle:?} not in {p:#?}"
    );
    p.into_iter().find(|p| p.message.contains(needle)).unwrap()
}

const GRADE: &str = "post/grade.wgsl";

#[test]
fn the_sample_passes() {
    let pack = sample();
    let report = check(&pack).unwrap();
    let labels: Vec<_> = report.stages.iter().map(|s| s.label.as_str()).collect();
    assert_eq!(
        labels,
        ["sky.wgsl", "Terrain", "Colour grade", "post/vignette.wgsl"]
    );
    assert!(report.stages.iter().all(|s| s.expressions > 0));
    let post = &report.stages[2];
    assert!(post.texture_reads >= 2, "the entry point samples and loads");
    let sha = pack.sha256().unwrap();
    assert_eq!(sha.len(), 64);
    assert_eq!(sha, sample().sha256().unwrap());
}

#[test]
fn a_syntax_error_points_at_its_line() {
    let p = one(
        GRADE,
        "override saturation: f32 = 1.1;\noverride tonemap: u32 = 1u;\nfn post(input: PostInput) -> vec4<f32> {\n    return input.color +;\n}\n",
        "expected",
    );
    assert_eq!((p.file.as_str(), p.line), (GRADE, 4), "{p}");
    assert!(p.to_string().starts_with("post/grade.wgsl:4:"), "{p}");
}

#[test]
fn the_stage_function_must_have_its_signature() {
    let header = "override saturation: f32 = 1.1;\noverride tonemap: u32 = 1u;\n";
    one(
        GRADE,
        &format!("{header}fn post(input: PostInput) -> vec3<f32> {{ return input.color.rgb; }}\n"),
        "`fn post(PostInput) -> vec3<f32>` should be `fn post(input: PostInput) -> vec4<f32>`",
    );
    one(
        GRADE,
        &format!("{header}fn grade(input: PostInput) -> vec4<f32> {{ return input.color; }}\n"),
        "defines no `fn post(input: PostInput) -> vec4<f32>`",
    );
}

#[test]
fn a_pack_declares_no_bindings_entry_points_or_horizon_names() {
    let body = "override saturation: f32 = 1.1;\noverride tonemap: u32 = 1u;\nfn post(input: PostInput) -> vec4<f32> { return input.color; }\n";
    one(
        GRADE,
        &format!("@group(3) @binding(9) var extra: texture_2d<f32>;\n{body}"),
        "declares the binding `extra`",
    );
    one(
        GRADE,
        &format!(
            "{body}@fragment fn main() -> @location(0) vec4<f32> {{ return vec4<f32>(1.0); }}\n"
        ),
        "declares the entry point `main`",
    );
    one(
        GRADE,
        &format!("{body}fn horizon_mine() {{}}\n"),
        "`horizon_mine`: names starting with horizon_",
    );
}

#[test]
fn the_options_sizing_the_shadow_map_need_no_override() {
    let manifest = |keys: &str| {
        let text = String::from_utf8(sample().files[MANIFEST].clone()).unwrap();
        let text = text.replace("distance = 96.0", &format!("distance = 96.0\n{keys}"));
        format!(
            "{text}\n[[options]]\nid = \"shadow_size\"\nname = \"Size\"\ntype = \"choice\"\n\
             values = [\"1024\", \"2048\"]\ndefault = \"1024\"\n"
        )
    };
    let mut files = sample().files;
    files.insert(
        MANIFEST.into(),
        manifest("resolution_option = \"shadow_size\"\n").into_bytes(),
    );
    let report = check(&Pack::from_files(files).unwrap()).unwrap();
    let shadow = report.shadow.unwrap();
    assert_eq!(shadow.resolution_option.as_deref(), Some("shadow_size"));
    one(
        MANIFEST,
        &manifest(""),
        "option \"shadow_size\": no stage declares `override shadow_size: u32`",
    );
}

#[test]
fn overrides_match_the_options() {
    let post = "fn post(input: PostInput) -> vec4<f32> { return input.color; }\n";
    one(
        GRADE,
        &format!("override saturation: i32 = 1;\noverride tonemap: u32 = 1u;\n{post}"),
        "option \"saturation\" is a float, so its override is `f32`",
    );
    one(
        GRADE,
        &format!(
            "override saturation: f32 = 1.1;\noverride tonemap: u32 = 1u;\noverride extra: f32;\n{post}"
        ),
        "`override extra` has no default and no [[options]] entry",
    );
    let p = one(
        GRADE,
        &format!("override saturation: f32 = 1.1;\n{post}"),
        "option \"tonemap\": no stage declares `override tonemap: u32`",
    );
    assert_eq!(p.file, MANIFEST);
}

#[test]
fn validation_errors_point_into_the_pack() {
    // A compute-only barrier in a fragment stage passes the parser, not the validator.
    let p = one(
        GRADE,
        "override saturation: f32 = 1.1;\noverride tonemap: u32 = 1u;\n\
         fn post(input: PostInput) -> vec4<f32> {\n    \
         workgroupBarrier();\n    \
         return input.color;\n}\n",
        "forbidden at this stage",
    );
    assert_eq!(p.file, GRADE, "{p}");
}

#[test]
fn texture_reads_have_a_budget() {
    let reads: String = (0..70)
        .map(|i| format!("    c += textureLoad(previous, vec2<i32>({i}, 0), 0);\n"))
        .collect();
    one(
        GRADE,
        &format!(
            "override saturation: f32 = 1.1;\noverride tonemap: u32 = 1u;\nfn post(input: PostInput) -> vec4<f32> {{\n    var c = input.color;\n{reads}    return c;\n}}\n"
        ),
        "texture reads; a stage may write 64",
    );
}

#[test]
fn reads_follows_the_stage_function_through_its_calls() {
    let material = |text: &str| {
        // The sample's `[reflection] option` is an override of the material.
        let text = format!("override reflections: bool = true;\n{text}");
        let mut files = sample().files;
        files.insert("material.wgsl".into(), text.into_bytes());
        let report = check(&Pack::from_files(files).unwrap()).unwrap();
        let stage = report.stages.into_iter().find(|s| s.label == "Terrain");
        stage.unwrap()
    };
    let sample = material(
        "fn material(input: MaterialInput) -> Surface {\n    return Surface(input.albedo);\n}\n",
    );
    assert!(!sample.reads("reflection"));
    let mirror = material(
        "fn mirrored(uv: vec2<f32>) -> vec4<f32> {\n    return textureSampleLevel(reflection, reflection_sampler, uv, 0.0);\n}\nfn material(input: MaterialInput) -> Surface {\n    return Surface(mix(input.albedo, mirrored(input.screen), 0.5));\n}\n",
    );
    assert!(mirror.reads("reflection"));
    assert!(mirror.reads("reflection_sampler"));
    assert!(!mirror.reads("opaque_color"));
}

/// A stage's `files` come before its own file, and no other stage sees them.
#[test]
fn a_stages_files_are_its_own() {
    let mut pack = sample();
    pack.files.insert(
        "only_post.wgsl".into(),
        b"fn only_post() -> f32 { return 1.0; }\n".to_vec(),
    );
    pack.manifest.post[0].files.push("only_post.wgsl".into());
    let file = pack.manifest.post[0].file.clone();
    let post = horizon_shader_api::compose(&pack, horizon_shader_api::StageKind::Post, &file);
    let names: Vec<_> = post.parts.iter().map(|p| p.0.as_str()).collect();
    let (extra, own) = (
        names.iter().position(|n| *n == "only_post.wgsl").unwrap(),
        names.iter().position(|n| *n == file).unwrap(),
    );
    assert!(extra < own, "{names:?}");
    let material = pack.manifest.material.clone().unwrap().file;
    let other =
        horizon_shader_api::compose(&pack, horizon_shader_api::StageKind::Material, &material);
    assert!(other.parts.iter().all(|p| p.0 != "only_post.wgsl"));
    assert!(pack.manifest.files().contains(&"only_post.wgsl"));
    check(&pack).unwrap();
}

#[test]
fn a_vertex_stage_checks_alone_and_joins_the_material() {
    let mut files = sample().files;
    let manifest = String::from_utf8(files[MANIFEST].clone()).unwrap();
    files.insert(
        MANIFEST.into(),
        format!("{manifest}\n[vertex]\nfile = \"wave.wgsl\"\n").into_bytes(),
    );
    files.insert(
        "wave.wgsl".into(),
        b"fn vertex(input: VertexInput) -> vec3<f32> {
    return input.position + vec3(0.0, sin(frame.time) * f32(input.material), 0.0);
}
"
        .to_vec(),
    );
    let report = check(&Pack::from_files(files).unwrap()).unwrap();
    let source = |kind| {
        let stage = report.stages.iter().find(|s| s.kind == kind).unwrap();
        stage.source.contains("fn vertex(input: VertexInput)")
    };
    assert!(source(StageKind::Vertex));
    assert!(source(StageKind::Material));
}

#[test]
fn a_material_reads_its_block_texture() {
    let mut files = sample().files;
    let pack = Pack::from_files(files.clone()).unwrap();
    let file = pack.manifest.material.as_ref().unwrap().file.clone();
    files.insert(
        file,
        b"override reflections: bool = true;
fn material(input: MaterialInput) -> Surface {
    let lod = block_texture_lod() + block_texture_size().x;
    return Surface(block_texture(input.uv) + block_texture_level(input.uv, lod));
}
"
        .to_vec(),
    );
    check(&Pack::from_files(files).unwrap()).unwrap();
}

#[test]
fn only_a_material_that_calls_set_surface_needs_its_target() {
    let calls = |pack: &Pack| {
        let report = check(pack).unwrap();
        let material = report.stages.iter().find(|s| s.kind == StageKind::Material);
        horizon_shader_api::stages::calls_set_surface(&material.unwrap().source)
    };
    let mut files = sample().files;
    let pack = Pack::from_files(files.clone()).unwrap();
    assert!(!calls(&pack));
    let material = pack.manifest.material.as_ref().unwrap().file.clone();
    files.insert(
        material,
        b"override reflections: bool = true;
fn material(input: MaterialInput) -> Surface {
    set_surface(input.normal, 0.5, 7.0);
    return Surface(input.albedo);
}
"
        .to_vec(),
    );
    assert!(calls(&Pack::from_files(files).unwrap()));
}

/// The sample with `[displacement]` added to its manifest and `material.wgsl` replaced.
fn displaced(material: &str) -> Result<horizon_shader_api::Report, Vec<Problem>> {
    let mut files = sample().files;
    let manifest = String::from_utf8(files[MANIFEST].clone()).unwrap() + "\n[displacement]\n";
    files.insert(MANIFEST.into(), manifest.into_bytes());
    let own = String::from_utf8(files["material.wgsl"].clone()).unwrap() + material;
    files.insert("material.wgsl".into(), own.into_bytes());
    check(&Pack::from_files(files).unwrap())
}

#[test]
fn displacement_is_checked_with_its_signature() {
    // Depth below the face by the LabPBR height: 1/8 of a block at the bottom of the relief.
    let good = "fn displacement(input: DisplacementInput) -> f32 { return (1.0 - input.height) * 0.125; }\n";
    assert!(displaced(good).is_ok(), "{:#?}", displaced(good).err());
    let missing = displaced("").expect_err("should fail");
    assert!(
        missing.iter().any(|p| p
            .message
            .contains("defines no `fn displacement(input: DisplacementInput) -> f32`")),
        "{missing:#?}"
    );
    let wrong = displaced(
        "fn displacement(input: DisplacementInput) -> vec3<f32> { return input.normal; }\n",
    )
    .expect_err("should fail");
    assert!(
        wrong.iter().any(|p| p
            .message
            .contains("`fn displacement(DisplacementInput) -> vec3<f32>` should be")),
        "{wrong:#?}"
    );
}

/// A pack of a post pass and a tonemap stage, with `extra` appended to the manifest.
fn post_pack(
    post: &str,
    tonemap: &str,
    extra: &str,
) -> Result<horizon_shader_api::Report, Vec<Problem>> {
    let manifest = format!(
        "schema = 1\nid = \"p\"\nname = \"P\"\nversion = \"1\"\napi = \"0.2\"\n\
         [[post]]\nfile = \"post.wgsl\"\n[tonemap]\nfile = \"tonemap.wgsl\"\n{extra}"
    );
    let files = [
        (MANIFEST, manifest.as_str()),
        ("post.wgsl", post),
        ("tonemap.wgsl", tonemap),
    ]
    .map(|(n, t)| (n.to_owned(), t.as_bytes().to_vec()))
    .into();
    check(&Pack::from_files(files).unwrap())
}

const POST: &str = "fn post(input: PostInput) -> vec4<f32> { return input.color; }\n";
const TONEMAP: &str =
    "fn tonemap(input: TonemapInput) -> vec3<f32> { return input.color / frame.headroom; }\n";

#[test]
fn the_tonemap_stage_is_checked() {
    let report = post_pack(POST, TONEMAP, "[frame]\nblend = \"linear\"\n").unwrap();
    let kinds: Vec<_> = report.stages.iter().map(|s| s.kind).collect();
    assert_eq!(kinds, [StageKind::Post, StageKind::Tonemap]);
    // A wide-gamut pack passes the check (the game refuses it for now), and reads its space.
    let wide = "fn tonemap(input: TonemapInput) -> vec3<f32> {\n    \
                return vec3<f32>(dot(input.color, frame.luminance)) * f32(frame.primaries);\n}\n";
    let frame = "[frame]\nblend = \"linear\"\nprimaries = \"bt2020\"\n";
    let report = post_pack(POST, wide, frame).unwrap();
    assert_eq!(
        report.manifest.primaries,
        horizon_shader_api::Primaries::Bt2020
    );
    // The prelude's display fields and colour boundary are there to use.
    let source = &report.stages[1].source;
    assert!(source.contains("    black_level: f32,\n") && source.contains("fn horizon_decode("));
    let wrong = post_pack(
        POST,
        "fn tonemap(input: TonemapInput) -> vec4<f32> { return vec4<f32>(input.color, 1.0); }\n",
        "",
    )
    .expect_err("should fail");
    assert!(
        wrong.iter().any(|p| p.message.contains(
            "`fn tonemap(TonemapInput) -> vec4<f32>` should be `fn tonemap(input: TonemapInput) -> vec3<f32>`"
        )),
        "{wrong:#?}"
    );
    let named = post_pack(
        POST,
        &format!("fn horizon_decode_mine(c: vec3<f32>) -> vec3<f32> {{ return c; }}\n{TONEMAP}"),
        "",
    )
    .expect_err("should fail");
    assert!(
        named
            .iter()
            .any(|p| p.message.contains("`horizon_decode_mine`")),
        "{named:#?}"
    );
}

/// A pack of a material, a post pass and a tonemap stage.
fn post_pack_with_material(
    material: &str,
    post: &str,
    tonemap: &str,
) -> Result<horizon_shader_api::Report, Vec<Problem>> {
    let manifest = "schema = 1\nid = \"p\"\nname = \"P\"\nversion = \"1\"\napi = \"0.2\"\n\
                    [material]\nfile = \"material.wgsl\"\n[[post]]\nfile = \"post.wgsl\"\n\
                    [tonemap]\nfile = \"tonemap.wgsl\"\n";
    let files = [
        (MANIFEST, manifest),
        ("material.wgsl", material),
        ("post.wgsl", post),
        ("tonemap.wgsl", tonemap),
    ]
    .map(|(n, t)| (n.to_owned(), t.as_bytes().to_vec()))
    .into();
    check(&Pack::from_files(files).unwrap())
}

const MATERIAL: &str =
    "fn material(input: MaterialInput) -> Surface { return Surface(input.albedo); }\n";
const ENTITY: &str = "fn entity(input: EntityInput) -> Surface { return Surface(input.albedo); }\n";

fn refused(problems: &[Problem], name: &str) -> Vec<(String, usize)> {
    let needle = format!("constructs `{name}`: it is Horizon's and may gain fields");
    problems
        .iter()
        .filter(|p| p.message.contains(&needle))
        .map(|p| (p.file.clone(), p.line))
        .collect()
}

#[test]
fn a_pack_constructs_no_struct_horizon_hands_it() {
    let post = "fn post(input: PostInput) -> vec4<f32> {\n    \
                let moved = PostInput(input.uv * 0.5, input.pixel, input.color, input.depth);\n    \
                return moved.color;\n}\n";
    let p = post_pack(post, TONEMAP, "").expect_err("should fail");
    assert_eq!(
        refused(&p, "PostInput"),
        [("post.wgsl".into(), 2)],
        "{p:#?}"
    );

    // Inside a helper, at the module's level, and the frame too.
    let tonemap = "fn darker(c: vec3<f32>) -> TonemapInput {\n    \
                   return TonemapInput(vec2<f32>(0.0), vec2<f32>(0.0), c * 0.5);\n}\n\
                   fn tonemap(input: TonemapInput) -> vec3<f32> { return darker(input.color).color; }\n";
    let p = post_pack(POST, tonemap, "").expect_err("should fail");
    assert_eq!(
        refused(&p, "TonemapInput"),
        [("tonemap.wgsl".into(), 2)],
        "{p:#?}"
    );
    let fields: Vec<String> = horizon_shader_api::Frame::WGSL
        .fields
        .iter()
        .map(|f| format!("{}()", f.ty))
        .collect();
    let tonemap = format!(
        "const still = Frame({});\n\
         fn tonemap(input: TonemapInput) -> vec3<f32> {{ return input.color * still.headroom; }}\n",
        fields.join(", ")
    );
    let p = post_pack(POST, &tonemap, "").expect_err("should fail");
    assert_eq!(refused(&p, "Frame"), [("tonemap.wgsl".into(), 1)], "{p:#?}");

    let material = "fn material(input: MaterialInput) -> Surface {\n    \
                    let again = MaterialInput(input.albedo, input.tint, input.light, input.lightmap, \
                    input.position, input.uv, input.translucent, input.tags, input.material, input.screen, \
                    input.face, input.fluid, input.emission, input.normal, input.tangent, input.colored_light, \
                    input.displaced);\n    \
                    return Surface(again.albedo);\n}\n";
    let p = post_pack_with_material(material, POST, TONEMAP).expect_err("should fail");
    assert_eq!(
        refused(&p, "MaterialInput"),
        [("material.wgsl".into(), 2)],
        "{p:#?}"
    );

    // `surface_at`'s result: Horizon's helper builds it, the pack doesn't.
    let post = "fn post(input: PostInput) -> vec4<f32> {\n    \
                let s = SurfaceData(true, vec3<f32>(0.0, 1.0, 0.0), 0.5, 0.0);\n    \
                return input.color * s.smoothness * surface_at(vec2<i32>(input.pixel)).smoothness;\n}\n";
    let p = post_pack(post, TONEMAP, "").expect_err("should fail");
    assert_eq!(
        refused(&p, "SurfaceData"),
        [("post.wgsl".into(), 2)],
        "{p:#?}"
    );
}

#[test]
fn a_pack_reads_horizon_s_structs_passes_them_along_and_builds_its_surface() {
    // A name that merely contains a struct's, a comment and a field read are no construction.
    let post = "// PostInput(input.uv) would be refused\n\
                struct MyPostInput { color: vec4<f32> }\n\
                fn graded(input: PostInput) -> MyPostInput { return MyPostInput(input.color * frame.headroom); }\n\
                fn post(input: PostInput) -> vec4<f32> { return graded(input).color; }\n";
    let tonemap = "fn curve(input: TonemapInput) -> vec3<f32> { return input.color / (input.color + 1.0); }\n\
                   fn tonemap(input: TonemapInput) -> vec3<f32> { return curve(input); }\n";
    post_pack_with_material(MATERIAL, post, tonemap).unwrap();
    // The tonemap prelude has no `SurfaceData`: one of the pack's own is the pack's to build.
    let tonemap = "struct SurfaceData { gain: f32 }\n\
                   fn tonemap(input: TonemapInput) -> vec3<f32> { return input.color * SurfaceData(0.5).gain; }\n";
    post_pack_with_material(MATERIAL, POST, tonemap).unwrap();
    // Nor is an alias of the pack's own by that name.
    let tonemap = "alias SurfaceData = vec3<f32>;\n\
                   fn tonemap(input: TonemapInput) -> vec3<f32> { return SurfaceData(input.color.r, 0.0, 0.0); }\n";
    post_pack_with_material(MATERIAL, POST, tonemap).unwrap();
}

/// The entity stage's fixture, which the frame identity test draws with
/// (tools/test_frame_identity_parity.py).
#[test]
fn the_entity_fixture_passes() {
    let pack = Pack::read(Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/samples/entity-stage"
    )))
    .unwrap();
    let report = check(&pack).unwrap();
    let kinds: Vec<_> = report.stages.iter().map(|s| s.kind).collect();
    assert_eq!(kinds, [StageKind::Sky, StageKind::Entity]);
}

/// The fixture of a pack that fogs its stages itself, as vanilla does.
#[test]
fn the_own_fog_fixture_passes() {
    let pack = Pack::read(Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/samples/own-fog"
    )))
    .unwrap();
    let report = check(&pack).unwrap();
    let kinds: Vec<_> = report.stages.iter().map(|s| s.kind).collect();
    assert_eq!(kinds, [StageKind::Material, StageKind::Entity]);
    assert!(!report.material_fog);
}

/// A pack of the entity stage alone.
fn entity_pack(entity: &str) -> Result<horizon_shader_api::Report, Vec<Problem>> {
    let manifest = "schema = 1\nid = \"p\"\nname = \"P\"\nversion = \"1\"\napi = \"0.2\"\n\
                    [entity]\nfile = \"entity.wgsl\"\nname = \"Mobs\"\n";
    let files = [(MANIFEST, manifest), ("entity.wgsl", entity)]
        .map(|(n, t)| (n.to_owned(), t.as_bytes().to_vec()))
        .into();
    check(&Pack::from_files(files).unwrap())
}

#[test]
fn the_entity_stage_is_checked_like_the_material_s() {
    // It reads its input, the frame, the shadow maps and the coloured-light grid.
    let lit = "fn entity(input: EntityInput) -> Surface {\n    \
               let p = shadow_position(input.position);\n    \
               let sun = textureSampleCompareLevel(shadow_map, shadow_sampler, p.xy, p.z);\n    \
               let through = textureSampleCompareLevel(shadow_map_translucent, shadow_sampler, p.xy, p.z);\n    \
               let tinted = textureLoad(shadow_color, vec2<i32>(p.xy * vec2<f32>(textureDimensions(shadow_color))), 0);\n    \
               let lamp = block_light_color(input.position);\n    \
               let facing = max(dot(input.normal, frame.light_direction), 0.0) * input.shade;\n    \
               var light = input.lightmap * (1.0 + f32(input.full_light)) + lamp.rgb * lamp.a * input.light.x;\n    \
               light += input.colored_light + sun * through * tinted.rgb * facing * f32(frame.has_shadow);\n    \
               if input.kind == 4u || input.translucent { light *= 0.5 + input.uv.x * input.screen.y; }\n    \
               let base = mix(input.albedo.rgb * input.tint.rgb, input.overlay.rgb, input.overlay.a);\n    \
               return Surface(vec4<f32>(base * light, input.albedo.a * input.tint.a));\n}\n";
    let report = entity_pack(lit).unwrap();
    let stage = &report.stages[0];
    assert_eq!(
        (stage.kind, stage.label.as_str()),
        (StageKind::Entity, "Mobs")
    );
    assert!(stage.reads("shadow_map") && stage.reads("block_light_colors"));
    assert!(stage.expressions > 0 && stage.texture_reads >= 4);
    // No block texture and no picture behind the surface here: those are the material stage's.
    for theirs in [
        "block_texture(input.uv)",
        "textureLoad(opaque_color, vec2<i32>(0), 0)",
    ] {
        let p = entity_pack(&format!(
            "fn entity(input: EntityInput) -> Surface {{ return Surface({theirs}); }}\n"
        ))
        .expect_err("should fail");
        assert_eq!(p[0].file, "entity.wgsl", "{p:#?}");
    }

    let p = entity_pack("fn entity(input: EntityInput) -> vec4<f32> { return input.albedo; }\n")
        .expect_err("should fail");
    assert!(
        p[0].message.contains(
            "`fn entity(EntityInput) -> vec4<f32>` should be `fn entity(input: EntityInput) -> Surface`"
        ),
        "{p:#?}"
    );
    let p =
        entity_pack("fn mob(input: EntityInput) -> Surface { return Surface(input.albedo); }\n")
            .expect_err("should fail");
    assert!(
        p[0].message
            .contains("defines no `fn entity(input: EntityInput) -> Surface`"),
        "{p:#?}"
    );

    // Its limits are the material's.
    let reads: String = (0..70)
        .map(|i| format!("    c += textureLoad(shadow_color, vec2<i32>({i}, 0), 0);\n"))
        .collect();
    let p = entity_pack(&format!(
        "fn entity(input: EntityInput) -> Surface {{\n    var c = input.albedo;\n{reads}    return Surface(c);\n}}\n"
    ))
    .expect_err("should fail");
    assert!(
        p[0].message.contains("texture reads; a stage may write 64"),
        "{p:#?}"
    );

    // `EntityInput` may gain fields: read, not constructed.
    let built = "fn entity(input: EntityInput) -> Surface {\n    \
                 let again = EntityInput(input.albedo, input.tint, input.overlay, input.lightmap, input.shade, \
                 input.position, input.full_light, input.normal, input.kind, input.colored_light, \
                 input.translucent, input.light, input.uv, input.screen);\n    \
                 return Surface(again.albedo);\n}\n";
    let p = entity_pack(built).expect_err("should fail");
    assert_eq!(
        refused(&p, "EntityInput"),
        [("entity.wgsl".into(), 2)],
        "{p:#?}"
    );
}

/// A pack with `[ray_tracing]` whose post pass is `post`.
fn traced(post: &str) -> Result<horizon_shader_api::Report, Vec<Problem>> {
    post_pack(post, TONEMAP, "[ray_tracing]\n")
}

/// Every name `[ray_tracing]` brings, used.
const SHADOWED: &str = "fn post(input: PostInput) -> vec4<f32> {
    let p = view_position(input.uv, input.depth);
    let hit = trace(p, frame.sun_direction, frame.ray_distance);
    let open = f32(frame.has_ray_tracing) * f32(!trace_any(p, frame.sun_direction, 64.0));
    let lit = hit.albedo.rgb * hit.tint.rgb * hit.light.y * max(dot(hit.normal, frame.sun_direction), 0.0);
    // The hit's own light, and its normal map turned into world axes.
    let n = hit_normal_map(hit).xy * 2.0 - 1.0;
    let bent = hit.tangent.xyz * n.x + cross(hit.normal, hit.tangent.xyz) * hit.tangent.w * n.y;
    let rough = select(0.5, 1.0 - hit_specular_map(hit).r, block_pbr_maps());
    let own = hit.albedo.rgb * hit.emission * f32(hit.fluid + u32(hit.translucent)) * rough + bent;
    // The first water or glass on the way to the sun, and whether anything at all is.
    let glass = trace_layers(p, frame.sun_direction, 64.0, RAY_TRANSLUCENT);
    let any = trace_any_layers(p, frame.sun_direction, 64.0, RAY_SOLID | RAY_TRANSLUCENT);
    let tinted = glass.albedo.rgb * glass.albedo.a * f32(glass.translucent) * f32(any);
    return vec4<f32>(lit * open + own + tinted + hit.position * hit.uv.x * hit.distance, f32(hit.material + hit.tags));
}
";

fn ray_queries(module: &naga::Module) -> bool {
    module.types.iter().any(|(_, t)| {
        matches!(
            t.inner,
            naga::TypeInner::RayQuery { .. } | naga::TypeInner::AccelerationStructure { .. }
        )
    })
}

fn valid(module: &naga::Module, capabilities: naga::valid::Capabilities) -> bool {
    naga::valid::Validator::new(naga::valid::ValidationFlags::all(), capabilities)
        .validate(module)
        .is_ok()
}

#[test]
fn a_traced_post_stage_is_checked_without_the_scene_and_with_it() {
    let report = traced(SHADOWED).unwrap();
    let post = &report.stages[0];
    // What every device takes: no ray query in it.
    assert!(!ray_queries(&post.module));
    let plain = naga::valid::Capabilities::default();
    assert!(valid(&post.module, plain));
    // What a device that traces takes, and only it.
    let scene = post.scene.as_ref().expect("the stage against the scene");
    assert!(ray_queries(scene));
    assert!(valid(scene, plain | naga::valid::Capabilities::RAY_QUERY));
    assert!(!valid(scene, plain));
    assert!(report.stages[1].scene.is_none(), "the tonemap stage");
    // The budgets count the scene's sprite reads.
    let untraced = post_pack(POST, TONEMAP, "").unwrap();
    assert!(untraced.stages[0].scene.is_none());
    let reads = traced(POST).unwrap().stages[0].texture_reads;
    assert!(reads > untraced.stages[0].texture_reads, "{reads}");
    // A pack that didn't ask has none of the names.
    let problems = post_pack(SHADOWED, TONEMAP, "").unwrap_err();
    assert_eq!(
        (problems[0].file.as_str(), problems[0].line),
        ("post.wgsl", 3)
    );
}

/// Spec #16011 item 5: turning its rays off is the pack's choice. A bool option's `override` may
/// guard `trace`: the stage is taken with the option off and on, without the scene and with it.
#[test]
fn a_pack_option_may_turn_its_rays_off() {
    let post = "override rt_shadows: bool = true;
fn post(input: PostInput) -> vec4<f32> {
    if !rt_shadows || frame.has_ray_tracing == 0u || input.depth == 0.0 {
        return input.color;
    }
    let hit = trace(view_position(input.uv, input.depth), frame.sun_direction, 64.0);
    return vec4<f32>(input.color.rgb * select(1.0, 0.5, hit.distance >= 0.0), input.color.a);
}
";
    let option = "[ray_tracing]\n[[options]]\nid = \"rt_shadows\"\nname = \"Ray traced shadows\"\n\
                  type = \"bool\"\ndefault = true\n";
    let report = post_pack(post, TONEMAP, option).unwrap();
    let stage = &report.stages[0];
    let plain = naga::valid::Capabilities::default();
    let scene = (stage.scene.as_ref()).map(|m| (m, plain | naga::valid::Capabilities::RAY_QUERY));
    for (module, capabilities) in [(&stage.module, plain)].into_iter().chain(scene) {
        let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), capabilities)
            .validate(module)
            .unwrap();
        for on in [0.0, 1.0] {
            let constants = [("rt_shadows".to_owned(), on)].into_iter().collect();
            let (specialized, _) =
                naga::back::pipeline_constants::process_overrides(module, &info, None, &constants)
                    .unwrap();
            assert!(valid(&specialized, capabilities), "rt_shadows = {on}");
        }
    }
    assert!(stage.scene.as_ref().is_some_and(ray_queries));
}

#[test]
fn a_traced_pack_names_no_ray_query_of_its_own() {
    let own = "// A ray_query in a comment is no use of one.
fn post(input: PostInput) -> vec4<f32> {
    var q: ray_query;
    rayQueryInitialize(&q, horizon_ray_scene, RayDesc(0u, 0xffu, 0.0, 1.0, input.color.rgb, input.color.rgb));
    return input.color;
}
";
    let problems = traced(own).unwrap_err();
    let found: Vec<_> = problems
        .iter()
        .map(|p| {
            assert!(p.message.contains("traces rays with `trace`"), "{p}");
            let name = p.message.split('`').nth(1).unwrap();
            (p.file.as_str(), p.line, p.column, name)
        })
        .collect();
    assert_eq!(
        found,
        [
            ("post.wgsl", 3, 12, "ray_query"),
            ("post.wgsl", 4, 5, "rayQueryInitialize"),
            ("post.wgsl", 4, 47, "RayDesc"),
        ]
    );
    // Block comments nest: the outer one ends at its own `*/`, and what follows it counts.
    let nested = format!("/* outer /* inner */ ray_query é */\n{POST}");
    traced(&nested).unwrap();
    let after = format!("/* outer /* inner */ */ alias Q = ray_query;\n{POST}");
    let problems = traced(&after).unwrap_err();
    assert!(
        problems[0]
            .to_string()
            .starts_with("post.wgsl:1:35: `ray_query`"),
        "{}",
        problems[0]
    );
    let enabled = format!("enable wgpu_ray_query;\n{POST}");
    let problems = traced(&enabled).unwrap_err();
    assert!(
        problems[0]
            .to_string()
            .starts_with("post.wgsl:1:8: `wgpu_ray_query`")
    );
    // The tonemap stage's files are the pack's too.
    let tonemap = "alias Scene = acceleration_structure;\n\
                   fn tonemap(input: TonemapInput) -> vec3<f32> { return input.color; }\n";
    let problems = post_pack(POST, tonemap, "[ray_tracing]\n").unwrap_err();
    assert!(
        problems[0]
            .to_string()
            .starts_with("tonemap.wgsl:1:15: `acceleration_structure`")
    );
    // `RayHit` may gain fields, like the stage inputs: a pack can't make up the sprite a hit's
    // maps are read from either.
    let built = "fn post(input: PostInput) -> vec4<f32> {
    let hit = RayHit(1.0, vec3<f32>(0.0), vec3<f32>(0.0), vec2<f32>(0.0), input.color, input.color,
        1.0, 1.0, vec2<f32>(0.0), 0u, 0u, 0.0, 0u, false, false, vec4<f32>(0.0), 7u);
    return hit_normal_map(hit);
}
";
    let problems = traced(built).unwrap_err();
    assert_eq!(refused(&problems, "RayHit"), [("post.wgsl".to_owned(), 2)]);
}

/// A 0.2 pack with one stage, `[{stage}]` with `keys`: its `{stage}.wgsl` holds `body`, and a
/// shared `helpers.wgsl`.
fn coverage_pack(
    stage: &str,
    keys: &str,
    body: &str,
    helpers: &str,
) -> Result<horizon_shader_api::Report, Vec<Problem>> {
    let manifest = format!(
        "schema = 1\nid = \"p\"\nname = \"P\"\nversion = \"1\"\napi = \"0.2\"\n\
         shared = [\"helpers.wgsl\"]\n[{stage}]\nfile = \"{stage}.wgsl\"\n{keys}"
    );
    let file = format!("{stage}.wgsl");
    let files = [
        (MANIFEST, manifest.as_str()),
        ("helpers.wgsl", helpers),
        (&file, body),
    ]
    .map(|(n, t)| (n.to_owned(), t.as_bytes().to_vec()))
    .into();
    check(&Pack::from_files(files).unwrap())
}

#[test]
fn a_discard_reachable_from_material_or_entity_needs_pack_coverage() {
    // Through a shared helper, two calls deep, inside a loop.
    let helpers = "fn cut(a: f32) {\n    for (var i = 0; i < 2; i++) {\n        \
                   if a < 0.3 {\n            discard;\n        }\n    }\n}\n\
                   fn holes(a: f32) -> f32 {\n    cut(a);\n    return a;\n}\n";
    let none = "fn unused() {}\n";
    for (stage, input, plain) in [
        ("material", "MaterialInput", MATERIAL),
        ("entity", "EntityInput", ENTITY),
    ] {
        let direct = format!(
            "fn {stage}(input: {input}) -> Surface {{\n    \
             if input.albedo.a < 0.3 {{\n        discard;\n    }}\n    \
             return Surface(input.albedo);\n}}\n"
        );
        let through = format!(
            "fn {stage}(input: {input}) -> Surface {{\n    \
             return Surface(vec4(input.albedo.rgb, holes(input.albedo.a)));\n}}\n"
        );
        let refused = |keys: &str, body: &str, helpers: &str| {
            let p = coverage_pack(stage, keys, body, helpers).expect_err("should fail");
            let p: Vec<_> = p
                .into_iter()
                .filter(|p| {
                    p.message
                        .contains(&format!("`discard` reachable from `{stage}`"))
                })
                .collect();
            assert_eq!(p.len(), 1, "{p:#?}");
            assert!(p[0].message.contains("coverage = \"pack\""), "{p:#?}");
            (p[0].file.clone(), p[0].line)
        };
        let own_file = format!("{stage}.wgsl");
        assert_eq!(refused("", &direct, none), (own_file, 3));
        assert_eq!(refused("", &through, helpers), ("helpers.wgsl".into(), 4));
        let engine = "coverage = \"engine\"\n";
        assert_eq!(
            refused(engine, &through, helpers),
            ("helpers.wgsl".into(), 4)
        );
        let own = "coverage = \"pack\"\n";
        coverage_pack(stage, own, &direct, none).unwrap();
        coverage_pack(stage, own, &through, helpers).unwrap();
        // A helper with a discard that the stage never calls cuts no hole.
        coverage_pack(stage, "", plain, helpers).unwrap();
    }
}

#[test]
fn a_traced_pack_s_own_hit_helper_is_refused_as_its_own_trace_is() {
    let names = [
        "trace_any",
        "trace_layers",
        "RAY_TRANSLUCENT",
        "RAY_ENTITY",
        "hit_normal_map",
        "block_pbr_maps",
    ];
    for name in names {
        let own = format!(
            "fn {name}(c: vec4<f32>) -> vec4<f32> {{ return c; }}
             fn post(input: PostInput) -> vec4<f32> {{ return {name}(input.color); }}
"
        );
        let problems: Vec<String> = traced(&own)
            .unwrap_err()
            .iter()
            .map(Problem::to_string)
            .collect();
        let redefined = format!("redefinition of `{name}`");
        assert!(
            problems
                .iter()
                .any(|p| p.starts_with("post.wgsl:1:") && p.contains(&redefined)),
            "{problems:#?}"
        );
        // Without the key the name is the pack's.
        post_pack(&own, TONEMAP, "").unwrap();
    }
    // In a material stage that traces, as its `trace` is; one that doesn't keeps the name.
    let own = "fn hit_specular_map(c: vec4<f32>) -> vec4<f32> { return c; }
";
    let quiet = format!("{own}{MATERIAL}");
    traced_material(&quiet, "", &[]).unwrap();
    let tracing = format!("{own}{REFLECTING}");
    let problems = traced_material(&tracing, "", &[]).unwrap_err();
    assert!(
        problems[0]
            .to_string()
            .contains("redefinition of `hit_specular_map`"),
        "{problems:#?}"
    );
}

/// A pack with `[ray_tracing]` and the material stage `material` (no post pass), its
/// manifest's other `keys` and `files` added.
fn traced_material(
    material: &str,
    keys: &str,
    files: &[(&str, &str)],
) -> Result<horizon_shader_api::Report, Vec<Problem>> {
    let manifest = format!(
        "schema = 1\nid = \"p\"\nname = \"P\"\nversion = \"1\"\napi = \"0.2\"\n\
         [material]\nfile = \"material.wgsl\"\n[ray_tracing]\n{keys}"
    );
    let files = [(MANIFEST, manifest.as_str()), ("material.wgsl", material)]
        .into_iter()
        .chain(files.iter().copied())
        .map(|(n, t)| (n.to_owned(), t.as_bytes().to_vec()))
        .collect();
    check(&Pack::from_files(files).unwrap())
}

/// Water that mirrors what a ray off it hits.
const REFLECTING: &str = "fn material(input: MaterialInput) -> Surface {
    let lit = input.albedo * input.tint * vec4<f32>(input.lightmap, 1.0);
    if !input.translucent {
        return Surface(lit);
    }
    let hit = trace(input.position, reflect(normalize(input.position), input.normal), 64.0);
    let shadowed = trace_any(input.position, frame.sun_direction, 64.0);
    // What it hits shines by its own light, dimmed by how rough its specular map says it is.
    let rough = 1.0 - hit_specular_map(hit).r * hit_normal_map(hit).b;
    let glow = hit.albedo.rgb * hit.emission * rough * f32(hit.fluid + 1u) + hit.tangent.xyz * 0.0;
    let seen = select(frame.sky_color.rgb, hit.albedo.rgb * hit.tint.rgb + glow, hit.distance >= 0.0);
    return Surface(vec4<f32>(mix(lit.rgb, seen, 0.5) * select(1.0, 0.5, shadowed), lit.a));
}
";

#[test]
fn a_traced_material_stage_is_checked_without_the_scene_and_with_it() {
    let report = traced_material(REFLECTING, "", &[]).unwrap();
    let material = &report.stages[0];
    assert!(!ray_queries(&material.module));
    let plain = naga::valid::Capabilities::default();
    assert!(valid(&material.module, plain));
    let scene = material
        .scene
        .as_ref()
        .expect("the stage against the scene");
    assert!(ray_queries(scene));
    assert!(valid(scene, plain | naga::valid::Capabilities::RAY_QUERY));
    assert!(!valid(scene, plain));
    // The renderer builds its terrain from the source, the directive first.
    let source = material.scene_source.as_deref().unwrap();
    assert!(source.starts_with("enable wgpu_ray_query;\n"));
    assert!(!material.source.contains("enable wgpu_ray_query"));
    // A stage that names only the functions that take layers traces as well.
    let layered = REFLECTING
        .replace("trace(input.position,", "trace_layers(input.position,")
        .replace("input.normal), 64.0)", "input.normal), 64.0, RAY_SOLID)")
        .replace(
            "trace_any(input.position, frame.sun_direction, 64.0)",
            "trace_any_layers(input.position, frame.sun_direction, 64.0, RAY_TRANSLUCENT)",
        );
    assert!(!layered.contains("trace(") && !layered.contains("trace_any("));
    let report = traced_material(&layered, "", &[]).unwrap();
    assert!(report.stages[0].scene.is_some());
}

#[test]
fn a_material_can_read_hdr_light_sources_and_ray_hit_looks_together() {
    let material = "fn material(input: MaterialInput) -> Surface {
    let hit = trace(input.position, frame.sun_direction, 64.0);
    let tier = f32(input.light_source_tier);
    return Surface(vec4(frame.full_lightmap * tier * hit.ao, 1.0));
}";
    let report = traced_material(material, "", &[]).unwrap();
    let stage = &report.stages[0];
    assert!(stage.light_source() && stage.traces_looks());
    assert!(!report.looks());
}

/// Ordinary `[[post]]` entries sharing a file are built from the first entry's, `files` and all,
/// as before compute passes: only compute entries are built per entry.
#[test]
fn ordinary_post_entries_sharing_a_file_build_as_the_first() {
    let manifest = "schema = 1\nid = \"p\"\nname = \"P\"\nversion = \"1\"\napi = \"0.2\"\n\
                    [[post]]\nfile = \"post.wgsl\"\nfiles = [\"first.wgsl\"]\n\
                    [[post]]\nfile = \"post.wgsl\"\nfiles = [\"second.wgsl\"]\n";
    let post = "fn post(input: PostInput) -> vec4<f32> { return input.color * tint(); }\n";
    let files = [
        (MANIFEST, manifest),
        ("post.wgsl", post),
        (
            "first.wgsl",
            "fn tint() -> vec4<f32> { return vec4<f32>(1.0); }\n",
        ),
        ("second.wgsl", "fn shade() -> f32 { return 0.5; }\n"),
    ]
    .map(|(n, t)| (n.to_owned(), t.as_bytes().to_vec()))
    .into();
    let pack = Pack::from_files(files).unwrap();
    // The second entry's `files` would leave `tint` undefined.
    let report = check(&pack).unwrap();
    let [a, b] = &report.stages[..] else {
        panic!("two stages")
    };
    assert_eq!(format!("{:?}", a.module), format!("{:?}", b.module));
    let composed = horizon_shader_api::compose(&pack, StageKind::Post, "post.wgsl").text;
    assert!(composed.contains("fn tint") && !composed.contains("fn shade"));
}

#[test]
fn a_material_stage_that_doesn_t_trace_is_built_as_before() {
    let quiet = "// Not traced: trace and trace_any only in /* nested /* comments */ trace */.\n\
                 fn material(input: MaterialInput) -> Surface { return Surface(input.albedo); }\n";
    let report = traced_material(quiet, "", &[]).unwrap();
    assert!(report.stages[0].scene.is_none() && report.stages[0].scene_source.is_none());
    // Its composition before `trace` reached the material stage, from the same pack with a
    // post pass (which `[ray_tracing]` needed then), but for the frame's fog fields since.
    let manifest = "schema = 1\nid = \"p\"\nname = \"P\"\nversion = \"1\"\napi = \"0.2\"\n\
                    [material]\nfile = \"material.wgsl\"\n[[post]]\nfile = \"post.wgsl\"\n\
                    [tonemap]\nfile = \"tonemap.wgsl\"\n[ray_tracing]\n";
    let files = [
        (MANIFEST, manifest),
        ("material.wgsl", quiet),
        ("post.wgsl", POST),
        ("tonemap.wgsl", TONEMAP),
    ]
    .map(|(n, t)| (n.to_owned(), t.as_bytes().to_vec()))
    .into();
    let pack = Pack::from_files(files).unwrap();
    let composed = horizon_shader_api::compose(&pack, StageKind::Material, "material.wgsl");
    assert_eq!(
        horizon_shader_api::package::sha256_hex(composed.text.as_bytes()),
        "b60d3c32cdaa469cdff8b8b746769b88229fbd0ad0cc89a757a17b3d51cf1b49"
    );
    // Without the key `trace` is the pack's own name, in the material stage too.
    let own = "fn trace(c: vec4<f32>) -> vec4<f32> { return c; }\n\
               fn material(input: MaterialInput) -> Surface { return Surface(trace(input.albedo)); }\n";
    let manifest = "schema = 1\nid = \"p\"\nname = \"P\"\nversion = \"1\"\napi = \"0.2\"\n\
                    [material]\nfile = \"material.wgsl\"\n";
    let files = [(MANIFEST, manifest), ("material.wgsl", own)]
        .map(|(n, t)| (n.to_owned(), t.as_bytes().to_vec()))
        .into();
    check(&Pack::from_files(files).unwrap()).unwrap();
}

#[test]
fn a_ray_from_a_vertex_shader_is_refused() {
    let reached = |problems: &[Problem], name: &str| {
        let needle = format!("`{name}` reaches a trace function: it runs in a vertex shader");
        problems
            .iter()
            .filter(|p| p.message.contains(&needle))
            .map(|p| (p.file.clone(), p.line))
            .collect::<Vec<_>>()
    };
    // `displacement`, through a helper of its own.
    let displaced = format!(
        "{MATERIAL}fn deep(p: vec3<f32>) -> bool {{ return trace_any(p, vec3<f32>(0.0, 1.0, 0.0), 8.0); }}\n\
         fn displacement(input: DisplacementInput) -> f32 {{ return select(0.0, 0.1, deep(input.position)); }}\n"
    );
    let problems = traced_material(&displaced, "[displacement]\n", &[]).unwrap_err();
    assert_eq!(
        reached(&problems, "displacement"),
        [("material.wgsl".to_owned(), 3)],
        "{problems:#?}"
    );
    // The vertex stage's `vertex`, which the material's module holds.
    let vertex = "fn vertex(input: VertexInput) -> vec3<f32> {\n    \
                  let hit = trace(input.position, vec3<f32>(0.0, 1.0, 0.0), 8.0);\n    \
                  return input.position + hit.normal;\n}\n";
    let keys = "[vertex]\nfile = \"vertex.wgsl\"\n";
    let problems = traced_material(MATERIAL, keys, &[("vertex.wgsl", vertex)]).unwrap_err();
    assert_eq!(
        reached(&problems, "vertex"),
        [("vertex.wgsl".to_owned(), 1)],
        "{problems:#?}"
    );
    // The functions that take layers are no way around it.
    let layered = vertex.replace("8.0)", "8.0, RAY_TRANSLUCENT)");
    let layered = layered.replace("trace(", "trace_layers(");
    let problems = traced_material(MATERIAL, keys, &[("vertex.wgsl", &layered)]).unwrap_err();
    assert_eq!(
        reached(&problems, "vertex"),
        [("vertex.wgsl".to_owned(), 1)],
        "{problems:#?}"
    );
    let layered = displaced.replace("8.0)", "8.0, RAY_SOLID | RAY_TRANSLUCENT)");
    let layered = layered.replace("trace_any(", "trace_any_layers(");
    let problems = traced_material(&layered, "[displacement]\n", &[]).unwrap_err();
    assert_eq!(
        reached(&problems, "displacement"),
        [("material.wgsl".to_owned(), 3)],
        "{problems:#?}"
    );
    // Nor is the entity layer, in either function.
    let entities = displaced.replace("8.0)", "8.0, RAY_ENTITY)");
    let entities = entities.replace("trace_any(", "trace_any_layers(");
    let problems = traced_material(&entities, "[displacement]\n", &[]).unwrap_err();
    assert_eq!(
        reached(&problems, "displacement"),
        [("material.wgsl".to_owned(), 3)],
        "{problems:#?}"
    );
    let entities = vertex.replace("8.0)", "8.0, RAY_ENTITY)");
    let entities = entities.replace("trace(", "trace_layers(");
    let problems = traced_material(MATERIAL, keys, &[("vertex.wgsl", &entities)]).unwrap_err();
    assert_eq!(
        reached(&problems, "vertex"),
        [("vertex.wgsl".to_owned(), 1)],
        "{problems:#?}"
    );
    // The material's own rays beside both are its to trace.
    let both = format!(
        "{REFLECTING}fn displacement(input: DisplacementInput) -> f32 {{ return input.height; }}\n"
    );
    let plain = "fn vertex(input: VertexInput) -> vec3<f32> { return input.position; }\n";
    let keys = format!("{keys}[displacement]\n");
    traced_material(&both, &keys, &[("vertex.wgsl", plain)]).unwrap();
}

/// A post pass that reads the entity layer: what of an entity the sun's ray meets first.
const ENTITY_SHADOWED: &str = "fn post(input: PostInput) -> vec4<f32> {
    let p = view_position(input.uv, input.depth);
    let hit = trace_layers(p, frame.sun_direction, 64.0, RAY_SOLID | RAY_ENTITY);
    let mob = select(0.0, 1.0, hit.entity);
    let shadowed = f32(trace_any_layers(p, frame.sun_direction, 64.0, RAY_ENTITY));
    return vec4<f32>(hit.albedo.rgb * mob + hit_normal_map(hit).rgb * shadowed, 1.0);
}
";

/// The entity layer is only a pack's whose files name `RAY_ENTITY`, anywhere outside a comment:
/// its post pass is checked without the scene and with it (each translated to SPIR-V, HLSL and
/// Metal), and has `RayHit.entity`; another pack's has neither.
#[test]
fn the_entity_layer_is_the_pack_s_that_names_it() {
    let report = traced(ENTITY_SHADOWED).unwrap();
    let post = &report.stages[0];
    assert!(post.traces_entities());
    let plain = naga::valid::Capabilities::default();
    assert!(!ray_queries(&post.module) && valid(&post.module, plain));
    let scene = post.scene.as_ref().unwrap();
    assert!(valid(scene, plain | naga::valid::Capabilities::RAY_QUERY));
    // Dead code names it as well.
    let dead = format!("fn unused() -> u32 {{ return RAY_ENTITY; }}\n{SHADOWED}");
    assert!(traced(&dead).unwrap().stages[0].traces_entities());
    // A comment doesn't; nor does a pack without `[ray_tracing]`, which has no layers at all.
    let commented = format!("// RAY_ENTITY\n{SHADOWED}");
    assert!(!traced(&commented).unwrap().stages[0].traces_entities());
    assert!(post_pack(ENTITY_SHADOWED, TONEMAP, "").is_err());
    // `entity` is a field only where the layer is.
    let reads = SHADOWED.replace("hit.material + hit.tags", "u32(hit.entity)");
    let problems = traced(&reads).unwrap_err();
    assert!(problems[0].message.contains("entity"), "{problems:#?}");
    let named = format!("const LAYERS: u32 = RAY_ENTITY;\n{reads}");
    assert!(traced(&named).unwrap().stages[0].traces_entities());
}

/// A post pass that lights its point with the sources its cell lists, each tested with a ray.
const LIT: &str = "fn post(input: PostInput) -> vec4<f32> {
    let p = view_position(input.uv, input.depth);
    let near = ray_lights_near(p);
    var light = vec3<f32>(0.0);
    for (var i = 0u; i < near.y; i++) {
        let source = ray_light(ray_light_at(near.x + i));
        let to = source.position - p;
        if !trace_any(p, normalize(to), length(to)) {
            light += source.color * source.level;
        }
    }
    return vec4<f32>(input.color.rgb + light * f32(ray_light_count()), 1.0);
}
";

/// `[ray_tracing]` with the light sources (`lights = true`).
fn lit(post: &str) -> Result<horizon_shader_api::Report, Vec<Problem>> {
    post_pack(post, TONEMAP, "[ray_tracing]\nlights = true\n")
}

/// With `lights = true` a post pass that names the light functions is checked without the scene
/// and with it (each translated to SPIR-V, HLSL and Metal) and binds their buffer against the
/// scene; one that names none, or only in a comment, binds none. Without the key the names are
/// unknown, and the check says which key brings them.
#[test]
fn the_light_sources_are_the_keyed_pack_s_that_names_them() {
    let report = lit(LIT).unwrap();
    let post = &report.stages[0];
    assert!(post.traces_lights());
    let plain = naga::valid::Capabilities::default();
    assert!(!ray_queries(&post.module) && valid(&post.module, plain));
    let scene = post.scene.as_ref().unwrap();
    assert!(valid(scene, plain | naga::valid::Capabilities::RAY_QUERY));
    assert!(!lit(SHADOWED).unwrap().stages[0].traces_lights());
    let commented = format!("// ray_lights_near\n{SHADOWED}");
    assert!(!lit(&commented).unwrap().stages[0].traces_lights());
    // `RayLight` may gain fields: the pack reads it, never builds it.
    let built = "fn post(input: PostInput) -> vec4<f32> {\n    \
                 let l = RayLight(vec3<f32>(0.0), 1.0, vec3<f32>(1.0), 0u);\n    \
                 return vec4<f32>(l.color, 1.0);\n}\n";
    let problems = lit(built).unwrap_err();
    assert_eq!(
        refused(&problems, "RayLight"),
        [("post.wgsl".to_owned(), 2)]
    );
    for unkeyed in [traced(LIT), post_pack(LIT, TONEMAP, "")] {
        let problems: Vec<String> = unkeyed
            .unwrap_err()
            .iter()
            .map(Problem::to_string)
            .collect();
        assert!(
            problems
                .iter()
                .any(|p| p.contains("`lights = true` under `[ray_tracing]`")),
            "{problems:#?}"
        );
    }
}

/// Two `[[post]]` entries of one file with different `files`: each binds the light sources by
/// its own files, whichever comes first.
#[test]
fn post_entries_of_one_file_bind_the_light_sources_by_their_own_files() {
    let post = "fn post(input: PostInput) -> vec4<f32> {\n    \
                return vec4<f32>(shade(view_position(input.uv, input.depth)), 1.0);\n}\n";
    let reads =
        "fn shade(p: vec3<f32>) -> vec3<f32> { return vec3<f32>(f32(ray_lights_near(p).y)); }\n";
    let plain = "fn shade(p: vec3<f32>) -> vec3<f32> { return p; }\n";
    for (first, second, lit) in [
        ("a.wgsl", "b.wgsl", [true, false]),
        ("b.wgsl", "a.wgsl", [false, true]),
    ] {
        let manifest = format!(
            "schema = 1\nid = \"p\"\nname = \"P\"\nversion = \"1\"\napi = \"0.2\"\n\
             [ray_tracing]\nlights = true\n\
             [[post]]\nname = \"one\"\nfile = \"post.wgsl\"\nfiles = [\"{first}\"]\n\
             [[post]]\nname = \"two\"\nfile = \"post.wgsl\"\nfiles = [\"{second}\"]\n\
             [tonemap]\nfile = \"tonemap.wgsl\"\n"
        );
        let files = [
            (MANIFEST, manifest.as_str()),
            ("post.wgsl", post),
            ("a.wgsl", reads),
            ("b.wgsl", plain),
            ("tonemap.wgsl", TONEMAP),
        ]
        .map(|(n, t)| (n.to_owned(), t.as_bytes().to_vec()))
        .into();
        let report = check(&Pack::from_files(files).unwrap()).unwrap();
        let posts = report.stages.iter().filter(|s| s.kind == StageKind::Post);
        let got: Vec<bool> = posts.map(|s| s.traces_lights()).collect();
        assert_eq!(got, lit, "{first} first");
    }
}

/// A light function of the pack's own (`ray_light` in the post pass and in a material stage
/// that traces nothing) stays the pack's without `lights = true`: the pack checks, its material
/// isn't built against the scene, and every stage composes byte for byte as the same pack with
/// the helper named otherwise, with the scene and without it. With the key the name is taken.
#[test]
fn a_pack_s_own_ray_light_stays_its_own_without_the_key() {
    let helper = "fn ray_light(c: vec4<f32>) -> vec4<f32> { return c * 0.5; }\n";
    let post = format!(
        "{helper}{}",
        SHADOWED
            .replace("return vec4<f32>(", "return ray_light(vec4<f32>(")
            .replace(
                "f32(hit.material + hit.tags));",
                "f32(hit.material + hit.tags)));"
            )
    );
    let material = format!(
        "{helper}fn material(input: MaterialInput) -> Surface {{ return Surface(ray_light(input.albedo)); }}\n"
    );
    let pack = |name: &str, keys: &str| {
        let manifest = format!(
            "schema = 1\nid = \"p\"\nname = \"P\"\nversion = \"1\"\napi = \"0.2\"\n\
             [material]\nfile = \"material.wgsl\"\n[[post]]\nfile = \"post.wgsl\"\n\
             [tonemap]\nfile = \"tonemap.wgsl\"\n[ray_tracing]\n{keys}"
        );
        let files = [
            (MANIFEST, manifest),
            ("material.wgsl", material.replace("ray_light", name)),
            ("post.wgsl", post.replace("ray_light", name)),
            ("tonemap.wgsl", TONEMAP.to_owned()),
        ]
        .map(|(n, t)| (n.to_owned(), t.into_bytes()))
        .into();
        Pack::from_files(files).unwrap()
    };
    let (own, other) = (pack("ray_light", ""), pack("pack_light", ""));
    let report = check(&own).unwrap();
    let material_stage = report.stages.iter().find(|s| s.kind == StageKind::Material);
    assert!(material_stage.unwrap().scene.is_none());
    assert!(report.stages.iter().all(|s| !s.traces_lights()));
    for (kind, file) in [
        (StageKind::Post, "post.wgsl"),
        (StageKind::Material, "material.wgsl"),
    ] {
        use horizon_shader_api::{compose, compose_scene};
        for build in [compose, compose_scene] {
            let renamed = build(&other, kind, file)
                .text
                .replace("pack_light", "ray_light");
            assert_eq!(build(&own, kind, file).text, renamed, "{kind:?}");
        }
    }
    let problems: Vec<String> = check(&pack("ray_light", "lights = true\n"))
        .unwrap_err()
        .iter()
        .map(Problem::to_string)
        .collect();
    assert!(
        problems
            .iter()
            .any(|p| p.contains("redefinition of `ray_light`")),
        "{problems:#?}"
    );
    // The key is set: no hint to set it.
    assert!(
        !problems.iter().any(|p| p.contains("lights = true")),
        "{problems:#?}"
    );
}

/// With `lights = true` a material stage that names a light function reads them (binding 80),
/// and a vertex stage that reaches one is refused as a ray from it is: the shadow map draws
/// with the vertex stage, so the sources are read only where the player's camera draws.
#[test]
fn the_material_reads_the_light_sources_and_the_vertex_stage_doesn_t() {
    let lit = "fn material(input: MaterialInput) -> Surface {\n    \
               return Surface(input.albedo * f32(ray_light_count()));\n}\n";
    let report = traced_material(lit, "lights = true\n", &[]).unwrap();
    assert!(report.stages[0].traces_lights());
    let vertex = "fn vertex(input: VertexInput) -> vec3<f32> {\n    \
                  return input.position * f32(ray_light_count());\n}\n";
    let keys = "lights = true\n[vertex]\nfile = \"vertex.wgsl\"\n";
    let problems = traced_material(MATERIAL, keys, &[("vertex.wgsl", vertex)]).unwrap_err();
    let needle = "`vertex` reaches the light sources: it runs in a vertex shader";
    assert!(
        problems
            .iter()
            .any(|p| p.file == "vertex.wgsl" && p.message.contains(needle)),
        "{problems:#?}"
    );
}

/// Pin the post and material WGSL of a pack that reads `hit.tint` and `hit.emission` and leaves
/// the entity layer off. The miss-only builds keep their old hashes; the scene builds add looks.
#[test]
fn a_traced_pack_that_doesn_t_name_ray_entity_composes_as_before() {
    let manifest = "schema = 1\nid = \"p\"\nname = \"P\"\nversion = \"1\"\napi = \"0.2\"\n\
                    [material]\nfile = \"material.wgsl\"\n[[post]]\nfile = \"post.wgsl\"\n\
                    [tonemap]\nfile = \"tonemap.wgsl\"\n[ray_tracing]\n";
    let files = [
        (MANIFEST, manifest),
        ("material.wgsl", REFLECTING),
        ("post.wgsl", SHADOWED),
        ("tonemap.wgsl", TONEMAP),
    ]
    .map(|(n, t)| (n.to_owned(), t.as_bytes().to_vec()))
    .into();
    let pack = Pack::from_files(files).unwrap();
    let stages = [
        (StageKind::Post, "post.wgsl"),
        (StageKind::Material, "material.wgsl"),
    ];
    let hashes: Vec<String> = stages
        .into_iter()
        .flat_map(|(kind, file)| {
            use horizon_shader_api::{compose, compose_scene};
            [compose(&pack, kind, file), compose_scene(&pack, kind, file)]
        })
        .map(|c| horizon_shader_api::package::sha256_hex(c.text.as_bytes()))
        .collect();
    assert_eq!(
        hashes,
        [
            "799b33836c979da0988245348c9d7ed9bb42b16f499794e7dcfc711b9fadc7cd",
            "dccdd1f28f0ea7624e8a4c006f80196bf2311c61dc116e29ae33878c97ac0a06",
            "e646501b8bcd9097579a8bcb14e11710439d85617dfc61b044bde4e0490c8681",
            "6c25d87e7bd62b1d8ba9fccc35d6ef6426a845e344e3bf45c197afdf1bdb7db2",
        ]
    );
}

/// The same pack with stages that read none of a terrain hit's look (`RayHit.tint`, `shade`,
/// `ao`) composes as before the scene kept looks, with the scene and without it.
#[test]
fn a_traced_pack_that_reads_no_hit_look_composes_as_before() {
    let manifest = "schema = 1\nid = \"p\"\nname = \"P\"\nversion = \"1\"\napi = \"0.2\"\n\
                    [material]\nfile = \"material.wgsl\"\n[[post]]\nfile = \"post.wgsl\"\n\
                    [tonemap]\nfile = \"tonemap.wgsl\"\n[ray_tracing]\n";
    let plain = |text: &str| text.replace("hit.tint", "hit.albedo");
    let (material, post) = (plain(REFLECTING), plain(SHADOWED));
    let files = [
        (MANIFEST, manifest),
        ("material.wgsl", &material),
        ("post.wgsl", &post),
        ("tonemap.wgsl", TONEMAP),
    ]
    .map(|(n, t)| (n.to_owned(), t.as_bytes().to_vec()))
    .into();
    let pack = Pack::from_files(files).unwrap();
    let report = check(&pack).unwrap();
    // Both stages still read `hit.emission`. Only `input.tint` needs the arena's looks.
    assert!(!report.ray_looks() && report.looks());
    let stages = [
        (StageKind::Post, "post.wgsl"),
        (StageKind::Material, "material.wgsl"),
    ];
    let hashes: Vec<String> = stages
        .into_iter()
        .flat_map(|(kind, file)| {
            use horizon_shader_api::{compose, compose_scene};
            [compose(&pack, kind, file), compose_scene(&pack, kind, file)]
        })
        .map(|c| horizon_shader_api::package::sha256_hex(c.text.as_bytes()))
        .collect();
    assert_eq!(
        hashes,
        [
            "4f8521322a8e7da2f5eef9345eafa5e0a03329d460e7ccfdb5dc7c30de5e0342",
            "cf83d4f5161d6cff5cce8e868f0e683533f088620d14a442b154b098c7f77682",
            "dd8aade96c75cdd11a2866b8ab9164c7a7d36355d317dc527de0b1eba4f4880c",
            "1e944762ceee869009feb9a014806d08aa635aa706fb2683cdf1b28e4b325880",
        ]
    );
}

/// The entity layer's bindings against the scene, the retained block entities' store the last of
/// them (`RAY_ENTITY_RETAINED_BINDING`), are the naming pack's alone: another traced pack's scene
/// has none of them.
#[test]
fn the_retained_block_entities_store_is_the_layer_s_last_binding() {
    use horizon_shader_api::stages::{
        GROUP, RAY_ENTITY_RETAINED_BINDING, RAY_ENTITY_STORE_BINDING,
    };
    let bindings = |post: &str| -> Vec<u32> {
        let report = traced(post).unwrap();
        let scene = report.stages[0].scene.clone().unwrap();
        let mut found: Vec<u32> = (scene.global_variables.iter())
            .filter_map(|(_, g)| g.binding.as_ref())
            .filter(|b| b.group == GROUP && b.binding >= RAY_ENTITY_STORE_BINDING)
            // The terrain's looks, after the layer's, are another trigger's.
            .filter(|b| b.binding != horizon_shader_api::stages::RAY_LOOKS_BINDING)
            .map(|b| b.binding)
            .collect();
        found.sort_unstable();
        found
    };
    assert_eq!(RAY_ENTITY_RETAINED_BINDING, 63);
    assert_eq!(bindings(ENTITY_SHADOWED), [60, 61, 62, 63]);
    assert_eq!(bindings(SHADOWED), [0u32; 0]);
}

/// `Report::looks`: a stage reads a field only the terrain's quad looks hold (`tint`, `shade`,
/// `ao`, `face`), by member on `MaterialInput` or `VertexInput`, through a `var` too; the
/// prelude reads none, nor do an entity stage's `tint`, a pack's own `shade`, and `emission`,
/// `shaded_tint` and `lit_tint`, which the vertex holds.
#[test]
fn looks_are_read_by_the_terrain_fields_alone() {
    let dir = |path: &str| Path::new(env!("CARGO_MANIFEST_DIR")).join(path);
    let report = |path: &str| check(&Pack::read(&dir(path)).unwrap()).unwrap();
    for (pack, looks) in [
        ("packs/golden_hour", false),
        ("packs/traced_shadows", false),
        ("packs/traced_water", false),
        ("packs/horizon_hdr", false),
        ("tests/samples/entity-stage", false),
        (
            "../../tools/fixtures/shaderpacks/frame_identity_terrain",
            true,
        ),
        (
            "../../tools/fixtures/shaderpacks/frame_identity_world",
            true,
        ),
        ("../../tools/fixtures/shaderpacks/entity_traced", false),
    ] {
        assert_eq!(report(pack).looks(), looks, "{pack}");
    }
    let material = |body: &str| {
        let mut files = sample().files;
        let text = format!(
            "override reflections: bool = true;\nfn material(input: MaterialInput) -> Surface {{\n{body}\n}}\n"
        );
        files.insert("material.wgsl".into(), text.into_bytes());
        check(&Pack::from_files(files).unwrap()).unwrap().looks()
    };
    assert!(!material(
        "    let shade = 0.5;\n    return Surface(input.albedo * shade);"
    ));
    assert!(material("    return Surface(input.albedo * input.tint);"));
    assert!(material(
        "    var i = input;\n    i.light.x = 1.0;\n    return Surface(vec4(f32(i.face)));"
    ));
    assert!(!material(
        "    var i = input;\n    i.light.x = 1.0;\n    \
         return Surface(i.shaded_tint * (1.0 + i.emission));"
    ));
    assert!(!material(
        "    var i = input;\n    \
         return Surface(vec4(i.lit_tint * input.lightmap, 1.0) * i.shaded_tint);"
    ));
    // Nor do Horizon's look's light sources, which the vertex and the sprite hold, and the
    // frame's full light.
    assert!(!material(
        "    let full = vec4(frame.full_lightmap * input.light_source_threshold, 1.0);\n    \
         let tier = f32(input.light_source_tier) * f32(input.light_source_whole);\n    \
         return Surface(input.albedo * full * tier);"
    ));
    // Nor does the frame's jitter.
    assert!(!material(
        "    let j = frame.jitter + frame.previous_jitter;\n    \
         return Surface(input.albedo * (1.0 + j.x * j.y));"
    ));
}

/// A pack whose files name either jitter field gets both in its `Frame`, and checks; one that
/// names neither gets neither.
#[test]
fn naming_either_jitter_declares_both() {
    let jitters = |body: &str| {
        let mut files = sample().files;
        let text = format!(
            "override reflections: bool = true;\nfn material(input: MaterialInput) -> Surface {{\n{body}\n}}\n"
        );
        files.insert("material.wgsl".into(), text.into_bytes());
        let report = check(&Pack::from_files(files).unwrap()).unwrap();
        let stage = (report.stages.iter()).find(|s| s.kind == StageKind::Material);
        let module = &stage.unwrap().module;
        let (_, frame) = (module.types.iter())
            .find(|(_, t)| t.name.as_deref() == Some("Frame"))
            .unwrap();
        let naga::TypeInner::Struct { members, .. } = &frame.inner else {
            panic!("Frame is a struct")
        };
        (members.iter())
            .filter_map(|m| m.name.clone())
            .filter(|n| n.contains("jitter"))
            .collect::<Vec<_>>()
    };
    let both = ["jitter", "previous_jitter"];
    for name in both {
        let body = format!("    return Surface(input.albedo + frame.{name}.x);");
        assert_eq!(jitters(&body), both, "{name}");
    }
    assert!(jitters("    return Surface(input.albedo);").is_empty());
}

/// The sample with its material stage's file `material`, its other `files` (the stage's own,
/// before its file) and, with `vertex`, a vertex stage of that file: whether it reads the looks.
fn reads_looks(material: &str, files: &[(&str, &str)], vertex: Option<&str>) -> bool {
    let mut pack = sample();
    // The sample's `[reflection]` is switched by this option of the material's.
    let material = format!("override reflections: bool = true;\n{material}");
    pack.files
        .insert("material.wgsl".into(), material.into_bytes());
    for (name, text) in files {
        pack.files.insert((*name).into(), text.as_bytes().to_vec());
    }
    let manifest = String::from_utf8(pack.files[MANIFEST].clone()).unwrap();
    let names: Vec<String> = files
        .iter()
        .map(|(name, _)| format!("\"{name}\""))
        .collect();
    let stage = "file = \"material.wgsl\"";
    let listed = format!("{stage}\nfiles = [{}]", names.join(", "));
    let mut manifest = manifest.replacen(stage, &listed, 1);
    if let Some(vertex) = vertex {
        manifest.push_str("\n[vertex]\nfile = \"vertex.wgsl\"\n");
        pack.files
            .insert("vertex.wgsl".into(), vertex.as_bytes().to_vec());
    }
    pack.files.insert(MANIFEST.into(), manifest.into_bytes());
    check(&Pack::from_files(pack.files).unwrap())
        .unwrap()
        .looks()
}

/// Every way a stage reaches a look field turns the looks on: through a pointer to the struct
/// (a helper taking `ptr<function, _>` or `ptr<private, _>`, on both structs), a copy passed to
/// a helper, a `let`, an array or a struct holding it, a swizzle after the member, and a helper
/// in another of the stage's files, called or not.
#[test]
fn looks_are_read_through_pointers_copies_and_other_files() {
    const PLAIN: &str =
        "fn material(input: MaterialInput) -> Surface {\n    return Surface(input.albedo);\n}\n";
    assert!(!reads_looks(PLAIN, &[], None));
    let forms = [
        (
            "a function pointer",
            "fn glow(p: ptr<function, MaterialInput>) -> f32 { return (*p).ao; }\n\
             fn material(input: MaterialInput) -> Surface {\n    var i = input;\n    \
             return Surface(vec4(glow(&i)));\n}\n",
        ),
        (
            "a private pointer",
            "var<private> kept: MaterialInput;\n\
             fn shade_of(p: ptr<private, MaterialInput>) -> f32 { return (*p).shade; }\n\
             fn material(input: MaterialInput) -> Surface {\n    kept = input;\n    \
             return Surface(vec4(shade_of(&kept)));\n}\n",
        ),
        (
            "a copy passed to a helper",
            "fn tinted(i: MaterialInput) -> vec4<f32> { return i.albedo * i.tint; }\n\
             fn material(input: MaterialInput) -> Surface {\n    return Surface(tinted(input));\n}\n",
        ),
        (
            "a let and a swizzle",
            "fn material(input: MaterialInput) -> Surface {\n    let i = input;\n    \
             return Surface(vec4(i.tint.rgb, 1.0));\n}\n",
        ),
        (
            "an array",
            "fn material(input: MaterialInput) -> Surface {\n    \
             var a = array<MaterialInput, 2>(input, input);\n    \
             return Surface(vec4(f32(a[1].face)));\n}\n",
        ),
        (
            "a struct holding it",
            "struct Wrap { inner: MaterialInput }\n\
             fn material(input: MaterialInput) -> Surface {\n    let w = Wrap(input);\n    \
             return Surface(vec4(w.inner.ao));\n}\n",
        ),
    ];
    let mut missed: Vec<&str> = (forms.iter())
        .filter(|(_, material)| !reads_looks(material, &[], None))
        .map(|(form, _)| *form)
        .collect();
    // In the stage's own other file, a helper the stage never calls.
    let lib = (
        "lib.wgsl",
        "fn lit(i: MaterialInput) -> f32 { return i.ao; }\n",
    );
    if !reads_looks(PLAIN, &[lib], None) {
        missed.push("another file");
    }
    // The vertex stage's input through a pointer.
    let vertex = "fn shade_of(p: ptr<function, VertexInput>) -> f32 { return (*p).shade; }\n\
                  fn vertex(input: VertexInput) -> vec3<f32> {\n    var i = input;\n    \
                  return input.position * shade_of(&i);\n}\n";
    if !reads_looks(PLAIN, &[], Some(vertex)) {
        missed.push("a vertex pointer");
    }
    assert!(missed.is_empty(), "missed: {missed:?}");
    let plain = "fn vertex(input: VertexInput) -> vec3<f32> { return input.position; }\n";
    assert!(!reads_looks(PLAIN, &[], Some(plain)));
}

/// What doesn't read a look field leaves the looks off, however near it comes: a post pass
/// that samples `shadow_color` (it holds the vertex colour, looks or none), and a traced post
/// pass reading a hit's `tint`, `shade`, `ao` and `emission` (the ray scene's own store holds
/// them, not the sections' looks).
#[test]
fn shadow_color_and_ray_hits_leave_the_looks_off() {
    let shadowed = "fn post(input: PostInput) -> vec4<f32> {\n    \
                    let size = vec2<f32>(textureDimensions(shadow_color));\n    \
                    let c = textureLoad(shadow_color, vec2<i32>(input.uv * size), 0);\n    \
                    return input.color * c;\n}\n";
    let report = post_pack(shadowed, TONEMAP, "[shadow]\n").unwrap();
    assert!(report.stages.iter().any(|s| s.reads("shadow_color")));
    assert!(!report.looks());
    let traced_hit = "fn post(input: PostInput) -> vec4<f32> {\n    \
                      let p = view_position(input.uv, input.depth);\n    \
                      let hit = trace(p, frame.sun_direction, frame.ray_distance);\n    \
                      let c = hit.tint.rgb * hit.shade * hit.ao + vec3(hit.emission);\n    \
                      return vec4<f32>(input.color.rgb * c, 1.0);\n}\n";
    assert!(!traced(traced_hit).unwrap().looks());
}

/// `MaterialInput.lit_tint` is only a pack's whose files name it, outside a comment, in the
/// material's own file or another of the pack's: only then is it declared, and does the renderer
/// give the terrain its varying (`CheckedStage::lit_tint`). Every other field stays where it was.
#[test]
fn lit_tint_is_the_pack_s_that_names_it() {
    let checked = |material: &str, lib: &str| {
        let manifest = "schema = 1\nid = \"t\"\nname = \"T\"\nversion = \"1\"\napi = \"0.2\"\n\
            shared = [\"lib.wgsl\"]\n[material]\nfile = \"material.wgsl\"\n";
        let files = [
            (MANIFEST, manifest),
            ("material.wgsl", material),
            ("lib.wgsl", lib),
        ]
        .map(|(n, t)| (n.to_owned(), t.as_bytes().to_vec()))
        .into();
        let mut report = check(&Pack::from_files(files).unwrap()).unwrap();
        report.stages.remove(0)
    };
    let plain = "fn material(input: MaterialInput) -> Surface {\n    \
                 // Not `input.lit_tint`: a comment names nothing.\n    \
                 return Surface(input.albedo);\n}\n";
    let lit = "fn material(input: MaterialInput) -> Surface {\n    \
               return Surface(input.albedo * vec4<f32>(input.lit_tint, 1.0));\n}\n";
    let helper = "fn unused(i: MaterialInput) -> vec3<f32> { return i.lit_tint; }\n";
    let declares = |stage: &horizon_shader_api::check::CheckedStage| {
        stage.source.contains("    lit_tint: vec3<f32>,\n")
    };
    let none = checked(plain, "");
    assert!(!none.lit_tint() && !declares(&none));
    for stage in [checked(lit, ""), checked(plain, helper)] {
        assert!(stage.lit_tint() && declares(&stage));
        assert!(!stage.shaded_tint() && !stage.light_source() && !stage.reads_looks());
    }
    // After the light-source fields and before `shaded_tint`, as the constructors pass them.
    let all = "fn material(input: MaterialInput) -> Surface {\n    \
               let t = f32(input.light_source_tier);\n    \
               return Surface(input.shaded_tint * vec4<f32>(input.lit_tint * t, 1.0));\n}\n";
    let stage = checked(all, "");
    assert!(stage.lit_tint() && stage.shaded_tint() && stage.light_source());
    let at = |field: &str| stage.source.find(&format!("    {field}: ")).unwrap();
    assert!(at("light_source_threshold") < at("lit_tint") && at("lit_tint") < at("shaded_tint"));
}

/// The ray scene's store of terrain looks is a stage's whose code reads a hit's `tint`, `shade`,
/// or `ao`: directly, through a copy, or in a helper of another file. Hits get
/// `shaded_tint` and `emission` from the vertex.
#[test]
fn a_hit_s_look_fields_bring_the_scene_s_looks() {
    let post = |body: &str| {
        format!(
            "fn post(input: PostInput) -> vec4<f32> {{\n    \
             let hit = trace(view_position(input.uv, input.depth), frame.sun_direction, 64.0);\n    \
             {body}\n}}\n"
        )
    };
    // Every pack has `lib.wgsl` beside its post pass, with `lit`.
    let looked = "fn lit(h: RayHit) -> f32 { return h.ao; }\n";
    let plain = "fn lit(h: RayHit) -> f32 { return h.emission; }\n";
    let ray_looks = |text: &str, lib: &str| {
        let manifest = "schema = 1\nid = \"p\"\nname = \"P\"\nversion = \"1\"\napi = \"0.2\"\n\
                        [[post]]\nfile = \"post.wgsl\"\nfiles = [\"lib.wgsl\"]\n\
                        [tonemap]\nfile = \"tonemap.wgsl\"\n[ray_tracing]\n";
        let files = [
            (MANIFEST, manifest),
            ("post.wgsl", text),
            ("lib.wgsl", lib),
            ("tonemap.wgsl", TONEMAP),
        ]
        .map(|(n, t)| (n.to_owned(), t.as_bytes().to_vec()))
        .into();
        let report = check(&Pack::from_files(files).unwrap()).unwrap();
        assert!(!report.looks(), "{text}");
        report.ray_looks()
    };
    let reads = [
        post("return vec4<f32>(hit.tint.rgb, 1.0);"),
        post("var h = hit; return vec4<f32>(h.tint.r);"),
        "fn component(p: ptr<function, RayHit>) -> f32 { return (*p).ao; }\n".to_owned()
            + &post("var h = hit; return vec4<f32>(component(&h));"),
        post("if input.uv.x > 0.5 { return vec4<f32>(lit(hit)); }\n    return input.color;"),
    ];
    for text in &reads {
        assert!(ray_looks(text, looked), "{text}");
    }
    // A hit held elsewhere than a local: in a pack struct, an array, a `var<private>`, or
    // returned from a pack helper and read by its caller.
    let held = [
        "struct Held { k: f32, hit: RayHit }\n".to_owned()
            + &post("let held = Held(1.0, hit);\n    return vec4<f32>(held.hit.tint.rgb, 1.0);"),
        post(
            "var hits = array<RayHit, 2>(hit, hit);\n    \
             return vec4<f32>(hits[u32(input.uv.x)].shade);",
        ),
        "var<private> kept: RayHit;\n".to_owned()
            + &post("kept = hit;\n    return vec4<f32>(kept.ao);"),
        "fn traced(p: vec3<f32>) -> RayHit { return trace(p, frame.sun_direction, 8.0); }\n"
            .to_owned()
            + &post(
                "let near = traced(view_position(input.uv, input.depth));\n    \
                 return vec4<f32>(near.ao + hit.distance);",
            ),
    ];
    for text in &held {
        assert!(ray_looks(text, plain), "{text}");
    }
    let none = [
        post("return input.color * hit.albedo;"),
        post("return vec4<f32>(hit.shaded_tint.rgb * hit.albedo.rgb, 1.0);"),
        post("return input.color * lit(hit);"),
        post("var h = hit; h.ao = 1.0; return vec4<f32>(h.distance);"),
    ];
    for text in &none {
        assert!(!ray_looks(text, plain), "{text}");
    }
    assert!(!ray_looks(&none[0], looked), "uncalled look helper");
}

/// A 0.2 pack with `[ray_tracing]` and the entity stage `entity` (no post pass, no material),
/// its manifest's other `keys` and `files` added.
fn traced_entity(
    entity: &str,
    keys: &str,
    files: &[(&str, &str)],
) -> Result<horizon_shader_api::Report, Vec<Problem>> {
    let manifest = format!(
        "schema = 1\nid = \"p\"\nname = \"P\"\nversion = \"1\"\napi = \"0.2\"\n\
         [entity]\nfile = \"entity.wgsl\"\n[ray_tracing]\n{keys}"
    );
    let files = [(MANIFEST, manifest.as_str()), ("entity.wgsl", entity)]
        .into_iter()
        .chain(files.iter().copied())
        .map(|(n, t)| (n.to_owned(), t.as_bytes().to_vec()))
        .collect();
    check(&Pack::from_files(files).unwrap())
}

/// Under `lights = true` an entity stage that names a light function, and no trace function, is
/// built against the scene with binding 80; without the key the names are unknown, and the check
/// says which key brings them. Its vertex shader is Horizon's, so no pack code reaches them there.
#[test]
fn the_entity_stage_reads_the_light_sources_it_names() {
    let lit = "fn entity(input: EntityInput) -> Surface {\n    \
               let near = ray_lights_near(input.position);\n    \
               let l = ray_light(ray_light_at(near.x));\n    \
               return Surface(input.albedo * l.level * f32(near.y));\n}\n";
    let report = traced_entity(lit, "lights = true\n", &[]).unwrap();
    let stage = &report.stages[0];
    assert_eq!(stage.kind, StageKind::Entity);
    assert!(stage.traces_lights());
    let plain = naga::valid::Capabilities::default();
    assert!(valid(
        stage.scene.as_ref().unwrap(),
        plain | naga::valid::Capabilities::RAY_QUERY
    ));
    assert!(
        !traced_entity(ENTITY_SUNLIT, "lights = true\n", &[])
            .unwrap()
            .stages[0]
            .traces_lights()
    );
    let problems: Vec<String> = traced_entity(lit, "", &[])
        .unwrap_err()
        .iter()
        .map(Problem::to_string)
        .collect();
    assert!(
        problems
            .iter()
            .any(|p| p.contains("`lights = true` under `[ray_tracing]`")),
        "{problems:#?}"
    );
}

/// A mob in the sun's shadow where a ray toward it meets terrain, its rim lit by what a ray
/// along its normal sees.
const ENTITY_SUNLIT: &str = "fn entity(input: EntityInput) -> Surface {
    let start = input.position + input.normal * 0.05;
    let shadowed = trace_any(start, frame.sun_direction, frame.ray_distance);
    let hit = trace(start, input.normal, 16.0);
    let rough = select(0.5, 1.0 - hit_specular_map(hit).r, block_pbr_maps());
    let rim = hit.albedo.rgb * hit.tint.rgb * rough * hit_normal_map(hit).b;
    let lit = input.albedo.rgb * input.tint.rgb * input.lightmap * select(1.0, 0.4, shadowed);
    return Surface(vec4<f32>(lit + rim * 0.1, input.albedo.a * input.tint.a));
}
";

/// An entity stage traces as the material stage does: checked without the scene and with it,
/// in a pack whose only stage it is.
#[test]
fn a_traced_entity_stage_is_checked_without_the_scene_and_with_it() {
    let report = traced_entity(ENTITY_SUNLIT, "", &[]).unwrap();
    let stage = &report.stages[0];
    assert_eq!(stage.kind, StageKind::Entity);
    assert!(!ray_queries(&stage.module));
    let plain = naga::valid::Capabilities::default();
    assert!(valid(&stage.module, plain));
    let scene = stage.scene.as_ref().expect("the stage against the scene");
    assert!(ray_queries(scene));
    assert!(valid(scene, plain | naga::valid::Capabilities::RAY_QUERY));
    assert!(!valid(scene, plain));
    assert!(!stage.traces_entities());
    assert!(stage.traces_looks());
    // The renderer builds its entities from the source, the directive first.
    let source = stage.scene_source.as_deref().unwrap();
    assert!(source.starts_with("enable wgpu_ray_query;\n"));
    assert!(!stage.source.contains("enable wgpu_ray_query"));
    // The entity layer, a mob's ray meeting the other mobs, where the pack names it.
    let layered = ENTITY_SUNLIT.replace(
        "trace_any(start, frame.sun_direction, frame.ray_distance)",
        "trace_any_layers(start, frame.sun_direction, frame.ray_distance, RAY_SOLID | RAY_ENTITY)",
    );
    let report = traced_entity(&layered, "", &[]).unwrap();
    assert!(report.stages[0].traces_entities());
    // Its files name no ray query of their own, as every traced stage's.
    let own = format!("fn mine(s: acceleration_structure) {{}}\n{ENTITY_SUNLIT}");
    let problems = traced_entity(&own, "", &[]).unwrap_err();
    assert!(
        problems[0].message.contains("`acceleration_structure`"),
        "{problems:#?}"
    );
    // Without the key the names aren't there.
    let p = entity_pack(ENTITY_SUNLIT).expect_err("should fail");
    assert_eq!(p[0].file, "entity.wgsl", "{p:#?}");
}

/// An entity stage that names no trace function, in a traced pack, composes as it did before
/// the entity stage traced, byte for byte, with the scene and without it.
#[test]
fn an_entity_stage_that_doesn_t_trace_is_built_as_before() {
    let quiet = "// Not traced: trace only in a comment.\n\
                 fn entity(input: EntityInput) -> Surface { return Surface(input.albedo); }\n";
    let manifest = "schema = 1\nid = \"p\"\nname = \"P\"\nversion = \"1\"\napi = \"0.2\"\n\
                    [entity]\nfile = \"entity.wgsl\"\n[[post]]\nfile = \"post.wgsl\"\n\
                    [tonemap]\nfile = \"tonemap.wgsl\"\n[ray_tracing]\n";
    let files = [
        (MANIFEST, manifest),
        ("entity.wgsl", quiet),
        ("post.wgsl", ENTITY_SHADOWED),
        ("tonemap.wgsl", TONEMAP),
    ]
    .map(|(n, t)| (n.to_owned(), t.as_bytes().to_vec()))
    .into();
    let pack = Pack::from_files(files).unwrap();
    let hashes: Vec<String> = [
        horizon_shader_api::compose(&pack, StageKind::Entity, "entity.wgsl"),
        horizon_shader_api::compose_scene(&pack, StageKind::Entity, "entity.wgsl"),
    ]
    .iter()
    .map(|c| horizon_shader_api::package::sha256_hex(c.text.as_bytes()))
    .collect();
    assert_eq!(hashes, [ENTITY_UNTRACED; 2]);
    // Nor does it trace in a pack whose only stage it is.
    let report = traced_entity(quiet, "", &[]).unwrap();
    assert!(report.stages[0].scene.is_none() && report.stages[0].scene_source.is_none());
}

/// The SHA-256 of the entity stage above, composed on main before entity tracing.
const ENTITY_UNTRACED: &str = "e7a641d743807343540ca08b7c56a10d606b123ab3c682be0398bb089bab23ca";

/// An entity stage that reads the light sources can't have its own `fn normalize`: Horizon's
/// entity vertex code shares its module and would call it. One that only traces keeps its own
/// builtin-named helpers, as on main.
#[test]
fn a_lit_entity_stage_can_t_take_a_builtin_s_name() {
    let own = "fn normalize(v: vec3<f32>) -> vec3<f32> { return v * f32(ray_light_count()); }
               fn entity(input: EntityInput) -> Surface {
                   return Surface(vec4<f32>(normalize(input.normal), 1.0));
}
";
    let problems = traced_entity(
        own,
        "lights = true
",
        &[],
    )
    .unwrap_err();
    let needle = "`fn normalize` takes the name of a WGSL builtin: an `[entity]` stage that reads";
    let found = problems.iter().find(|p| p.message.contains(needle));
    assert_eq!(
        found.map(|p| (p.file.as_str(), p.line)),
        Some(("entity.wgsl", 1)),
        "{problems:#?}"
    );
    let traced = "fn atanh(x: f32) -> f32 { return 0.5 * log((1.0 + x) / (1.0 - x)); }
                  fn entity(input: EntityInput) -> Surface {
                      let open = trace_any(input.position, input.normal, 16.0);
                      return Surface(input.albedo * atanh(f32(open) * 0.5));
}
";
    assert!(
        traced_entity(
            traced,
            "lights = true
",
            &[]
        )
        .is_ok()
    );
}

/// A pack without `[ray_tracing]` keeps a builtin-named helper whatever its comments say.
#[test]
fn a_comment_naming_binding_80_doesn_t_refuse_a_helper() {
    let material = "// as @binding(80) would
                    fn atanh(x: f32) -> f32 { return 0.5 * log((1.0 + x) / (1.0 - x)); }
                    fn material(input: MaterialInput) -> Surface {
                        return Surface(input.albedo * atanh(0.25));
}
";
    let report = post_pack_with_material(material, POST, TONEMAP);
    assert!(report.is_ok(), "{report:#?}");
}

/// A material stage that reads the light sources can't have its own `fn fract`: Horizon's
/// terrain vertex code shares its module and would call it. One that only traces keeps it.
#[test]
fn a_lit_material_stage_can_t_take_a_builtin_s_name() {
    let fract = "fn fract(a: f32) -> f32 { return a - floor(a); }\n";
    let lit = format!(
        "{fract}fn material(input: MaterialInput) -> Surface {{\n    \
         return Surface(input.albedo * fract(f32(ray_light_count())));\n}}\n"
    );
    let problems = traced_material(&lit, "lights = true\n", &[]).unwrap_err();
    let needle = "`fn fract` takes the name of a WGSL builtin: a `[material]` stage that reads";
    let found = problems.iter().find(|p| p.message.contains(needle));
    assert_eq!(found.map(|p| p.line), Some(1), "{problems:#?}");
    let traced = format!(
        "{fract}fn material(input: MaterialInput) -> Surface {{\n    \
         let open = trace_any(input.position, frame.sun_direction, 16.0);\n    \
         return Surface(input.albedo * fract(f32(open)));\n}}\n"
    );
    assert!(traced_material(&traced, "", &[]).is_ok());
}
