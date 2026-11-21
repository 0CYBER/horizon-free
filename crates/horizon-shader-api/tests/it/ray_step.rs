//! `[ray_tracing] step = true`: `trace_step` hands every candidate of one ray query to the pack's
//! own `ray_step`, with the pack's own `RayState`. Composed and checked without the scene and with
//! it (the check takes each through naga's SPIR-V, Metal and HLSL back ends); byte for byte as
//! before without the key or the names; and the check's refusals.
use horizon_shader_api::{
    MANIFEST, Pack, Problem, Report, StageKind, check, compose, compose_scene,
};

/// `trace_light` of #19249 written as a pack's step: the light that gets through, the product of
/// the translucent texels on the way, `mix(1, albedo.rgb * tint.rgb, albedo.a)`, and 0 behind a
/// solid surface; water and the two `skip` materials let it through untinted.
const LIGHT_STEP: &str = "struct RayState {
    light: vec3<f32>,
    skip: vec2<u32>,
}
fn ray_step(c: RayCandidate, state: ptr<function, RayState>) -> u32 {
    // A clear texel, as the surface's own pass draws it.
    if !candidate_stops(c) {
        return RAY_PASS;
    }
    let material = candidate_material(c);
    if material == (*state).skip.x || material == (*state).skip.y {
        return RAY_PASS;
    }
    if c.layer == RAY_SOLID {
        (*state).light = vec3<f32>(0.0);
        return RAY_END;
    }
    if candidate_fluid(c) == 1u {
        return RAY_PASS;
    }
    let albedo = candidate_albedo(c);
    (*state).light *= mix(vec3<f32>(1.0), albedo.rgb * candidate_tint(c).rgb, albedo.a);
    return RAY_PASS;
}
fn trace_light(origin: vec3<f32>, direction: vec3<f32>, max_distance: f32, skip: vec2<u32>) -> vec3<f32> {
    var state = RayState(vec3<f32>(1.0), skip);
    _ = trace_step(origin, direction, max_distance, RAY_SOLID | RAY_TRANSLUCENT, &state);
    return state.light;
}
";

/// A post pass that lights by `trace_light`.
const LIGHT_POST: &str = "fn post(input: PostInput) -> vec4<f32> {
    let p = view_position(input.uv, input.depth);
    let sun = trace_light(p, frame.sun_direction, 64.0, vec2<u32>(3u, 0xffffffffu));
    return vec4<f32>(input.color.rgb * sun, 1.0);
}
";

/// `trace_layers` and `trace_any_layers` as steps: today's stop rule, kept as the nearest hit, or
/// with `RAY_END` the first one found.
const LAYERS_STEP: &str = "struct RayState {
    any: bool,
}
fn ray_step(c: RayCandidate, state: ptr<function, RayState>) -> u32 {
    if !candidate_stops(c) {
        return RAY_PASS;
    }
    return select(RAY_STOP, RAY_STOP | RAY_END, (*state).any);
}
fn post(input: PostInput) -> vec4<f32> {
    let p = view_position(input.uv, input.depth);
    var nearest = RayState(false);
    let hit = trace_step(p, frame.sun_direction, 64.0, RAY_SOLID | RAY_TRANSLUCENT, &nearest);
    var first = RayState(true);
    let blocked = trace_step(p, frame.sun_direction, 64.0, RAY_SOLID, &first).distance >= 0.0;
    return vec4<f32>(hit.albedo.rgb * hit.light.y * f32(!blocked), f32(hit.material));
}
";

/// A spectral pack's state: sixteen bands, each dimmed by the glass it passes.
const SPECTRAL_STEP: &str = "const BANDS: u32 = 16u;
struct RayState {
    bands: array<f32, 16>,
}
fn ray_step(c: RayCandidate, state: ptr<function, RayState>) -> u32 {
    if !candidate_stops(c) {
        return RAY_PASS;
    }
    if c.layer != RAY_TRANSLUCENT {
        for (var i = 0u; i < BANDS; i++) {
            (*state).bands[i] = 0.0;
        }
        return RAY_END;
    }
    let albedo = candidate_albedo(c);
    for (var i = 0u; i < BANDS; i++) {
        let w = f32(i) / f32(BANDS - 1u);
        let r = mix(mix(albedo.b, albedo.g, saturate(w * 2.0)), albedo.r, saturate(w * 2.0 - 1.0));
        (*state).bands[i] *= mix(1.0, r, albedo.a);
    }
    return RAY_PASS;
}
fn material(input: MaterialInput) -> Surface {
    var state: RayState;
    for (var i = 0u; i < BANDS; i++) {
        state.bands[i] = 1.0;
    }
    _ = trace_step(input.position + input.normal * 0.02, frame.sun_direction, 64.0, RAY_SOLID | RAY_TRANSLUCENT, &state);
    var light = 0.0;
    for (var i = 0u; i < BANDS; i++) {
        light += state.bands[i] / f32(BANDS);
    }
    return Surface(vec4<f32>(input.albedo.rgb * light, input.albedo.a));
}
";

const TONEMAP: &str =
    "fn tonemap(input: TonemapInput) -> vec3<f32> { return input.color / frame.headroom; }\n";

/// A pack of `stages` (`(manifest table, file, text)`), with `[ray_tracing]` and its `keys`.
fn pack(stages: &[(&str, &str, &str)], keys: &str) -> Pack {
    let mut manifest =
        String::from("schema = 1\nid = \"p\"\nname = \"P\"\nversion = \"1\"\napi = \"0.2\"\n");
    for (table, file, _) in stages {
        manifest += &format!("{table}\nfile = \"{file}\"\n");
    }
    manifest += &format!("[ray_tracing]\n{keys}");
    let files = std::iter::once((MANIFEST.to_owned(), manifest.into_bytes()))
        .chain(
            (stages.iter()).map(|(_, file, text)| ((*file).to_owned(), text.as_bytes().to_vec())),
        )
        .collect();
    Pack::from_files(files).unwrap()
}

/// A post pass `post` and a tonemap stage, checked.
fn post(post: &str, keys: &str) -> Result<Report, Vec<Problem>> {
    let stages = [
        ("[[post]]", "post.wgsl", post),
        ("[tonemap]", "tonemap.wgsl", TONEMAP),
    ];
    check(&pack(&stages, keys))
}

/// A material stage `material`, and `extra` stages, checked.
fn material(
    material: &str,
    keys: &str,
    extra: &[(&str, &str, &str)],
) -> Result<Report, Vec<Problem>> {
    let mut stages = vec![("[material]", "material.wgsl", material)];
    stages.extend_from_slice(extra);
    check(&pack(&stages, keys))
}

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

/// Whether one of `problems` says `needle`.
fn says(problems: &[Problem], needle: &str) -> bool {
    problems.iter().any(|p| p.message.contains(needle))
}

/// `report`'s first stage: without the scene every device takes it, with it only one that traces.
fn both_ways(report: &Report) {
    let stage = &report.stages[0];
    let plain = naga::valid::Capabilities::default();
    assert!(!ray_queries(&stage.module) && valid(&stage.module, plain));
    let scene = stage.scene.as_ref().expect("the stage against the scene");
    assert!(ray_queries(scene));
    assert!(valid(scene, plain | naga::valid::Capabilities::RAY_QUERY));
    assert!(!valid(scene, plain));
    assert!(stage.traces_steps() && report.ray_steps());
}

/// `trace_light` as a pack's step composes and is checked (naga, SPIR-V, Metal 2.4, HLSL) without
/// the scene and with it, in a post pass and a material stage; its `candidate_tint` brings the
/// scene's looks, as `RayHit.tint` does.
#[test]
fn trace_light_is_a_pack_step() {
    let light_post = format!("{LIGHT_STEP}{LIGHT_POST}");
    let report = post(&light_post, "step = true\n").unwrap();
    both_ways(&report);
    assert!(report.stages[0].traces_looks());
    assert!(!report.stages[1].traces_steps(), "the tonemap stage");
    let lit = "fn material(input: MaterialInput) -> Surface {
    let sun = trace_light(input.position + input.normal * 0.02, frame.sun_direction, 64.0, vec2<u32>(0xffffffffu));
    return Surface(vec4<f32>(input.albedo.rgb * sun, input.albedo.a));
}
";
    let report = material(&format!("{LIGHT_STEP}{lit}"), "step = true\n", &[]).unwrap();
    both_ways(&report);
    let entity = "fn entity(input: EntityInput) -> Surface {
    let sun = trace_light(input.position + input.normal * 0.05, frame.sun_direction, 64.0, vec2<u32>(0xffffffffu));
    return Surface(vec4<f32>(input.albedo.rgb * input.tint.rgb * sun, input.albedo.a));
}
";
    let stages = [("[entity]", "entity.wgsl", &*format!("{LIGHT_STEP}{entity}"))];
    both_ways(&check(&pack(&stages, "step = true\n")).unwrap());
    let compute = "fn compute(input: ComputeInput) {
    let sun = trace_light(vec3<f32>(input.cell), frame.sun_direction, 64.0, vec2<u32>(0xffffffffu));
    light_out[input.cell.x] = sun.x;
}
";
    let manifest = "schema = 1\nid = \"p\"\nname = \"P\"\nversion = \"1\"\napi = \"0.2\"\n\
                    [[resource]]\nname = \"light_out\"\nbuffer = 16\nelement = \"f32\"\nkeep = \"view\"\n\
                    [[before_world]]\nfile = \"c.wgsl\"\ndispatch = [16, 1, 1]\n\
                    workgroup = [16, 1, 1]\nwrites = [\"light_out\"]\n\
                    [tonemap]\nfile = \"t.wgsl\"\n[ray_tracing]\nstep = true\n";
    let source = format!("{LIGHT_STEP}{compute}");
    let files = [
        (MANIFEST, manifest),
        ("c.wgsl", source.as_str()),
        ("t.wgsl", TONEMAP),
    ]
    .map(|(n, t)| (n.to_owned(), t.as_bytes().to_vec()))
    .into();
    both_ways(&check(&Pack::from_files(files).unwrap()).unwrap());
    // Without `candidate_tint`, no look store.
    let untinted = light_post.replace(" * candidate_tint(c).rgb", "");
    let report = post(&untinted, "step = true\n").unwrap();
    assert!(report.stages[0].traces_steps() && !report.stages[0].traces_looks());
}

/// `trace_layers` and `trace_any_layers` as steps, over every layer with the entity layer's.
#[test]
fn the_layer_helpers_are_pack_steps() {
    both_ways(&post(LAYERS_STEP, "step = true\n").unwrap());
    let entities = LAYERS_STEP.replace(
        "RAY_SOLID | RAY_TRANSLUCENT,",
        "RAY_SOLID | RAY_TRANSLUCENT | RAY_ENTITY,",
    );
    let report = post(&entities, "step = true\n").unwrap();
    both_ways(&report);
    assert!(report.stages[0].traces_entities());
}

/// The state is the pack's own type: sixteen bands, a vector by `alias`.
#[test]
fn the_state_is_the_pack_s_own_type() {
    both_ways(&material(SPECTRAL_STEP, "step = true\n", &[]).unwrap());
    let aliased = "alias RayState = vec4<f32>;
fn ray_step(c: RayCandidate, state: ptr<function, RayState>) -> u32 {
    *state *= select(1.0, 0.5, candidate_stops(c));
    return RAY_PASS;
}
fn post(input: PostInput) -> vec4<f32> {
    var state = vec4<f32>(1.0);
    _ = trace_step(vec3<f32>(0.0), frame.sun_direction, 16.0, RAY_TRANSLUCENT, &state);
    return input.color * state;
}
";
    both_ways(&post(aliased, "step = true\n").unwrap());
}

/// A pass that doesn't name the step's names, or only in a comment, composes byte for byte as in
/// the pack without the key, with the scene and without; and so does every other stage.
#[test]
fn without_its_names_a_stage_composes_as_without_the_key() {
    let traced = "// trace_step, RayCandidate, ray_step\n\
                  fn post(input: PostInput) -> vec4<f32> {\n    \
                  let hit = trace_layers(vec3<f32>(0.0), frame.sun_direction, 8.0, RAY_SOLID | RAY_ENTITY);\n    \
                  return input.color * hit.tint * f32(trace_any(vec3<f32>(0.0), frame.sun_direction, 8.0));\n}\n";
    let lit = "fn material(input: MaterialInput) -> Surface {\n    \
               let hit = trace(input.position, frame.sun_direction, 8.0);\n    \
               return Surface(input.albedo * hit.shaded_tint);\n}\n";
    let stages = [
        ("[[post]]", "post.wgsl", traced),
        ("[material]", "material.wgsl", lit),
        ("[tonemap]", "tonemap.wgsl", TONEMAP),
    ];
    for other in ["", "lights = true\n"] {
        let (keyed, plain) = (
            pack(&stages, &format!("{other}step = true\n")),
            pack(&stages, other),
        );
        for (kind, file) in [
            (StageKind::Post, "post.wgsl"),
            (StageKind::Material, "material.wgsl"),
            (StageKind::Tonemap, "tonemap.wgsl"),
        ] {
            for build in [compose, compose_scene] {
                assert_eq!(
                    build(&keyed, kind, file).text,
                    build(&plain, kind, file).text,
                    "{kind:?}"
                );
            }
        }
        let report = check(&keyed).unwrap();
        assert!(!report.ray_steps());
    }
}

/// Without the key the names are the pack's own; with it `trace_step` is Horizon's, and the
/// pack's own a redefinition. `RayState` and `ray_step` stay the pack's either way.
#[test]
fn the_names_are_the_keyed_pack_s() {
    let own = "struct RayState { a: f32 }
fn ray_step(a: f32) -> f32 { return a; }
fn trace_step(a: f32) -> f32 { return a; }
fn post(input: PostInput) -> vec4<f32> {
    return input.color * trace_step(ray_step(RayState(1.0).a));
}
";
    // A pack's own `trace_step` is not Horizon's: the renderer reads that it doesn't step.
    let report = post(own, "").unwrap();
    assert!(report.stages[0].scene.is_some());
    assert!(!report.stages[0].traces_steps() && !report.ray_steps());
    let problems = post(own, "step = true\n").unwrap_err();
    assert!(
        says(&problems, "redefinition of `trace_step`"),
        "{problems:#?}"
    );
    // Named without the key, or in a stage that can't trace: what to do about it.
    let problems = post(LAYERS_STEP, "").unwrap_err();
    assert!(
        says(&problems, "`step = true` under `[ray_tracing]`"),
        "{problems:#?}"
    );
    let tonemap = "fn tonemap(input: TonemapInput) -> vec3<f32> {
    return input.color * f32(RAY_STOP);
}
";
    let stages = [
        ("[[post]]", "post.wgsl", LAYERS_STEP),
        ("[tonemap]", "tonemap.wgsl", tonemap),
    ];
    let problems = check(&pack(&stages, "step = true\n")).unwrap_err();
    assert!(
        says(
            &problems,
            "only in the post, compute, `[[before_world]]`, material and entity stages"
        ),
        "{problems:#?}"
    );
}

/// The check's refusals: the pack's step and state missing or mistyped, a step that traces, a
/// pack that builds a candidate, a step reached from a vertex shader or reaching a discard the
/// engine's coverage forbids.
#[test]
fn the_check_refuses_a_step_it_can_t_run() {
    let refused = |text: &str, needle: &str| {
        let problems = post(text, "step = true\n").unwrap_err();
        assert!(says(&problems, needle), "{needle}: {problems:#?}");
        problems
    };
    let signature = "fn ray_step(c: RayCandidate, state: ptr<function, RayState>) -> u32";
    // No step, no state: the stage's own file is named, not Horizon's prelude.
    let problems = refused(
        &LAYERS_STEP.replace("fn ray_step(", "fn other_step("),
        signature,
    );
    assert_eq!(problems[0].file, "post.wgsl");
    let problems = refused(
        &LAYERS_STEP
            .replace(
                "struct RayState {\n    any: bool,\n}\n",
                "struct Other {\n    any: bool,\n}\n",
            )
            .replace("RayState(", "Other("),
        "defines `struct RayState`",
    );
    assert_eq!(problems[0].file, "post.wgsl");
    // Mistyped: its result, its state, its candidate.
    let returns = LAYERS_STEP
        .replace(
            "state: ptr<function, RayState>) -> u32",
            "state: ptr<function, RayState>) -> bool",
        )
        .replace("return RAY_PASS;", "return false;")
        .replace(
            "return select(RAY_STOP, RAY_STOP | RAY_END, (*state).any);",
            "return true;",
        );
    let problems = refused(
        &returns,
        "should be `fn ray_step(c: RayCandidate, state: ptr<function, RayState>) -> u32`",
    );
    assert_eq!(
        (problems[0].file.as_str(), problems[0].line),
        ("post.wgsl", 4)
    );
    refused(
        &LAYERS_STEP
            .replace(
                "state: ptr<function, RayState>) -> u32",
                "state: ptr<function, vec4<f32>>) -> u32",
            )
            .replace("(*state).any", "(*state).x > 0.0"),
        "`fn ray_step(RayCandidate, ptr<function, vec4<f32>>) -> u32` should be",
    );
    refused(
        &LAYERS_STEP
            .replace("fn ray_step(c: RayCandidate,", "fn ray_step(c: u32,")
            .replace("candidate_stops(c)", "c == 0u"),
        "`fn ray_step(u32, ptr<function, RayState>) -> u32` should be",
    );
    // A step that traces: itself (WGSL's recursion) or through another trace function.
    let nested = LAYERS_STEP.replace(
        "    if !candidate_stops(c) {\n        return RAY_PASS;",
        "    if !candidate_stops(c) || trace_any(vec3<f32>(0.0), vec3<f32>(1.0, 0.0, 0.0), 1.0) {\n        return RAY_PASS;",
    );
    refused(&nested, "`ray_step` reaches `trace_any`");
    let helper = "fn deeper(p: vec3<f32>) -> f32 { return trace_layers(p, vec3<f32>(1.0, 0.0, 0.0), 1.0, RAY_SOLID).distance; }\n";
    let deep = format!(
        "{helper}{}",
        LAYERS_STEP.replace(
            "if !candidate_stops(c) {",
            "if !candidate_stops(c) || deeper(vec3<f32>(c.distance)) > 0.0 {"
        )
    );
    refused(&deep, "`ray_step` reaches `trace_layers`");
    let recursive = LAYERS_STEP.replace(
        "    if !candidate_stops(c) {\n        return RAY_PASS;",
        "    var again = RayState(true);\n    if !candidate_stops(c) || trace_step(vec3<f32>(0.0), vec3<f32>(1.0, 0.0, 0.0), 1.0, RAY_SOLID, &again).distance > 0.0 {\n        return RAY_PASS;",
    );
    refused(
        &recursive,
        "can't reach `trace_step` (WGSL has no recursion)",
    );
    // A candidate is Horizon's, read and passed along, never built: it may gain fields.
    let built = LAYERS_STEP.replace(
        "    let hit = trace_step(",
        "    _ = ray_step(RayCandidate(1.0, RAY_SOLID), &nearest);\n    let hit = trace_step(",
    );
    refused(&built, "constructs `RayCandidate`");
    // Nor as a zero value: `RayCandidate()`, or a variable left without an initializer, alone, in
    // an array or at module scope.
    let at_call = "    let hit = trace_step(";
    for (zero, line) in [
        ("    _ = ray_step(RayCandidate(), &nearest);\n", 13),
        (
            "    var c: RayCandidate;\n    _ = ray_step(c, &nearest);\n",
            13,
        ),
        (
            "    var cs: array<RayCandidate, 2>;\n    _ = ray_step(cs[1], &nearest);\n",
            13,
        ),
        (
            "    _ = ray_step(array<RayCandidate, 2>()[0], &nearest);\n",
            13,
        ),
    ] {
        let zeroed = LAYERS_STEP.replace(at_call, &format!("{zero}{at_call}"));
        let problems = refused(&zeroed, "constructs `RayCandidate`");
        assert_eq!(
            (problems[0].file.as_str(), problems[0].line),
            ("post.wgsl", line)
        );
    }
    let wrapped = format!(
        "struct Hold {{ c: RayCandidate }}\n{}",
        LAYERS_STEP.replace(
            at_call,
            &format!("    _ = ray_step(Hold().c, &nearest);\n{at_call}")
        )
    );
    refused(&wrapped, "constructs `RayCandidate`");
    let global = format!(
        "var<private> held: RayCandidate;\n{}",
        LAYERS_STEP.replace(
            at_call,
            &format!("    _ = ray_step(held, &nearest);\n{at_call}")
        )
    );
    let problems = refused(&global, "constructs `RayCandidate`");
    assert_eq!(
        (problems[0].file.as_str(), problems[0].line),
        ("post.wgsl", 1)
    );
    // A candidate passed along, or copied into a `let` or initialized `var`, is read, not built.
    let passed = "struct RayState { n: u32 }
fn keep(c: RayCandidate) -> u32 { let d = c; var copy = c; return select(RAY_PASS, RAY_STOP, d.distance > 0.0 && copy.distance > 0.0); }
fn ray_step(c: RayCandidate, state: ptr<function, RayState>) -> u32 { return keep(c); }
fn post(input: PostInput) -> vec4<f32> {
    var s = RayState(0u);
    return input.color * trace_step(vec3<f32>(0.0), frame.sun_direction, 8.0, RAY_SOLID, &s).distance;
}
";
    assert!(post(passed, "step = true\n").is_ok());
    // The vertex stage's `vertex`, which the material's module holds, runs in a vertex shader.
    let vertex = format!(
        "{}fn vertex(input: VertexInput) -> vec3<f32> {{\n    \
         var state = RayState(false);\n    \
         return input.position + trace_step(input.position, vec3<f32>(0.0, 1.0, 0.0), 8.0, RAY_SOLID, &state).normal;\n}}\n",
        &LAYERS_STEP[..LAYERS_STEP.find("fn post(").unwrap()]
    );
    let lit = "fn material(input: MaterialInput) -> Surface { return Surface(input.albedo); }\n";
    let extra = [("[vertex]", "vertex.wgsl", vertex.as_str())];
    let problems = material(lit, "step = true\n", &extra).unwrap_err();
    assert!(
        says(
            &problems,
            "`vertex` reaches a trace function: it runs in a vertex shader"
        ),
        "{problems:#?}"
    );
    // A discard in the step is reached from `material`, though `trace_step` misses without the
    // scene: the engine's coverage refuses it.
    let discarding = SPECTRAL_STEP.replace("        return RAY_END;", "        discard;");
    let problems = material(&discarding, "step = true\n", &[]).unwrap_err();
    assert!(
        says(&problems, "`discard` reachable from `material`"),
        "{problems:#?}"
    );
}

/// Every marker of the step's text at once: a pack that names `shaded_tint`, `RAY_ENTITY`, the
/// light sources and `candidate_tint` composes and is checked with the scene and without it.
#[test]
fn a_step_composes_with_every_other_opt_in() {
    let all = format!(
        "{LIGHT_STEP}fn post(input: PostInput) -> vec4<f32> {{
    let p = view_position(input.uv, input.depth);
    let sun = trace_light(p, frame.sun_direction, 64.0, vec2<u32>(3u, 0xffffffffu));
    var state = RayState(vec3<f32>(1.0), vec2<u32>(0xffffffffu));
    let hit = trace_step(p, frame.sun_direction, 8.0, RAY_SOLID | RAY_ENTITY, &state);
    let lit = f32(ray_light_count()) * hit.shaded_tint.rgb * hit.tint.rgb;
    return vec4<f32>(input.color.rgb * sun * state.light + lit, 1.0);
}}
"
    );
    let report = post(&all, "lights = true\nstep = true\n").unwrap();
    both_ways(&report);
    let stage = &report.stages[0];
    assert!(stage.traces_entities() && stage.traces_lights() && stage.traces_looks());
}

/// Two post entries of one file, one listing a file that names nothing of the step: under the key
/// each is checked from the same sources as without it.
#[test]
fn post_entries_sharing_a_file_compose_as_without_the_key() {
    let traced = "fn post(input: PostInput) -> vec4<f32> {\n    \
                  return input.color * f32(trace_any(vec3<f32>(0.0), frame.sun_direction, 8.0));\n}\n";
    let helper = "fn helper(a: f32) -> f32 { return a * 2.0; }\n";
    let manifest = |keys: &str| {
        format!(
            "schema = 1\nid = \"p\"\nname = \"P\"\nversion = \"1\"\napi = \"0.2\"\n\
             [[post]]\nfile = \"post.wgsl\"\n[[post]]\nfile = \"post.wgsl\"\nfiles = [\"helper.wgsl\"]\n\
             [tonemap]\nfile = \"t.wgsl\"\n[ray_tracing]\n{keys}"
        )
    };
    let sources = |keys: &str| {
        let manifest = manifest(keys);
        let files = [
            (MANIFEST, manifest.as_str()),
            ("post.wgsl", traced),
            ("helper.wgsl", helper),
            ("t.wgsl", TONEMAP),
        ]
        .map(|(n, t)| (n.to_owned(), t.as_bytes().to_vec()))
        .into();
        let report = check(&Pack::from_files(files).unwrap()).unwrap();
        (report.stages.iter())
            .map(|s| (s.source.clone(), s.scene_source.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(sources("step = true\n"), sources(""));
}

/// Two post entries of one file under the key, only one of them listing a file that names the
/// step: each has `trace_step` by its own files, whichever comes first.
#[test]
fn post_entries_sharing_a_file_step_by_their_own_files() {
    let stepping = "struct RayState { n: u32 }
fn ray_step(c: RayCandidate, state: ptr<function, RayState>) -> u32 {
    return select(RAY_PASS, RAY_STOP, candidate_stops(c));
}
fn shade(p: vec3<f32>) -> f32 {
    var s = RayState(0u);
    return trace_step(p, frame.sun_direction, 8.0, RAY_SOLID, &s).distance;
}
";
    let plain = "fn shade(p: vec3<f32>) -> f32 { return 1.0; }\n";
    let pass =
        "fn post(input: PostInput) -> vec4<f32> { return input.color * shade(vec3<f32>(0.0)); }\n";
    for stepped_first in [true, false] {
        let entry =
            |helper: &str| format!("[[post]]\nfile = \"post.wgsl\"\nfiles = [\"{helper}\"]\n");
        let (first, second) = match stepped_first {
            true => (entry("stepping.wgsl"), entry("plain.wgsl")),
            false => (entry("plain.wgsl"), entry("stepping.wgsl")),
        };
        let manifest = format!(
            "schema = 1\nid = \"p\"\nname = \"P\"\nversion = \"1\"\napi = \"0.2\"\n{first}{second}\
             [tonemap]\nfile = \"t.wgsl\"\n[ray_tracing]\nstep = true\n"
        );
        let files = [
            (MANIFEST, manifest.as_str()),
            ("post.wgsl", pass),
            ("stepping.wgsl", stepping),
            ("plain.wgsl", plain),
            ("t.wgsl", TONEMAP),
        ]
        .map(|(n, t)| (n.to_owned(), t.as_bytes().to_vec()))
        .into();
        let report = check(&Pack::from_files(files).unwrap()).unwrap();
        let steps: Vec<bool> = (report.stages.iter())
            .filter(|s| s.kind == StageKind::Post)
            .map(|s| s.traces_steps())
            .collect();
        assert_eq!(steps, [stepped_first, !stepped_first]);
        assert!(report.ray_steps());
    }
}
