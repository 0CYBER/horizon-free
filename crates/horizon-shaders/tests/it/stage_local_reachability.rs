//! Original GLSL for issue #8406, exercised through the public translation API.
use std::io::{Cursor, Write};

use horizon_shaders::{
    GameInputs, GbuffersInputs, Macros, Pack, Program, Target, load_post, preprocess,
    read_properties, translate_composite, translate_gbuffers, translate_shadow,
};

const VERTEX: &str = "#version 120\nvoid main() { gl_Position = ftransform(); }\n";

fn pack(files: &[(&str, &str)]) -> Pack {
    let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (path, text) in files {
        archive
            .start_file(path, zip::write::SimpleFileOptions::default())
            .unwrap();
        archive.write_all(text.as_bytes()).unwrap();
    }
    Pack::from_zip_bytes(
        "original-reachability.zip",
        archive.finish().unwrap().into_inner(),
    )
    .unwrap()
}

fn fragment(source: &str) -> Result<Program, Vec<horizon_shaders::Error>> {
    let pack = pack(&[
        ("shaders/composite.vsh", VERTEX),
        ("shaders/composite.fsh", source),
    ]);
    let macros = Macros::standard();
    translate_composite(
        &preprocess(&pack, "composite.vsh", &macros),
        &preprocess(&pack, "composite.fsh", &macros),
        Target::Window,
    )
}

#[test]
fn unused_vertex_helper_in_a_shared_fragment_header_translates() {
    fragment("#version 120\nvec2 texel1() { return gl_MultiTexCoord1.xy; }\nvoid main() { gl_FragColor = vec4(0.5); }\n")
        .unwrap_or_else(|errors| panic!("unused vertex helper rejected: {errors:?}"));
}

#[test]
fn a_shared_helper_is_live_only_in_the_vertex_stage() {
    let pack = pack(&[
        (
            "shaders/lib/shared.glsl",
            "#define LIGHTMAP texel1\nvec2 texel1() { return gl_MultiTexCoord1.xy; }\n",
        ),
        (
            "shaders/composite.vsh",
            "#version 120\n#include \"/lib/shared.glsl\"\nvoid main() { gl_Position = ftransform(); gl_Position.xy += LIGHTMAP(); }\n",
        ),
        (
            "shaders/composite.fsh",
            "#version 120\n#include \"/lib/shared.glsl\"\nvoid main() { gl_FragColor = vec4(0.5); }\n",
        ),
    ]);
    let mut macros = Macros::standard();
    let (properties, notes) = read_properties(&pack, &mut macros);
    assert!(notes.is_empty(), "{notes:?}");
    let v = preprocess(&pack, "composite.vsh", &macros);
    let f = preprocess(&pack, "composite.fsh", &macros);
    translate_composite(&v, &f, Target::Window).unwrap();
    translate_gbuffers(&v, &f, Target::Buffers, &GbuffersInputs::default()).unwrap();
    translate_shadow(&v, &f, &GbuffersInputs::default()).unwrap();
    let post = load_post(&pack, "", &properties, &macros, &GameInputs::default());
    assert!(post.notes.is_empty(), "{:?}", post.notes);
    assert_eq!(post.programs.len(), 1);
    assert!(post.programs[0].program.is_some());
}

#[test]
fn required_overloads_prototypes_and_global_initializers_survive() {
    let live = r#"#version 120
float initial() { return 0.25; }
float factor = initial();
float scale(float value);
vec2 scale(vec2 value);
float scale(float value) { return value * factor; }
vec2 scale(vec2 value) { return value * factor; }
void main() { gl_FragColor = vec4(scale(1.0), scale(vec2(2.0)), 1.0); }
"#;
    fragment(live).unwrap();
    fragment(&format!(
        "{live}\nvec2 unused() {{ return gl_MultiTexCoord1.xy; }}\n"
    ))
    .unwrap();
}

#[test]
fn only_unreachable_call_chains_are_removed() {
    let helpers = "#version 120\nvec2 texel1() { return gl_MultiTexCoord1.xy; }\nvec4 relay() { return vec4(texel1(), 0.0, 1.0); }\n";
    fragment(&format!(
        "{helpers}void main() {{ gl_FragColor = vec4(0.5); }}\n"
    ))
    .unwrap();
    let errors = fragment(&format!(
        "{helpers}void main() {{ gl_FragColor = relay(); }}\n"
    ))
    .err()
    .expect("reachable vertex input must remain an error");
    assert!(
        errors
            .iter()
            .any(|e| e.message.contains("gl_MultiTexCoord1")),
        "{errors:?}"
    );
}

#[test]
fn retained_global_initializers_are_reachability_roots() {
    let errors = fragment("#version 120\nvec2 texel1() { return gl_MultiTexCoord1.xy; }\nvec2 saved = texel1();\nvoid main() { gl_FragColor = vec4(0.5); }\n")
        .err().expect("an initializer must not lose its required function");
    assert!(
        errors
            .iter()
            .any(|e| e.message.contains("gl_MultiTexCoord1")),
        "{errors:?}"
    );
}

#[test]
fn errors_after_removed_helpers_keep_the_include_file_and_line() {
    let pack = pack(&[
        ("shaders/composite.vsh", VERTEX),
        (
            "shaders/composite.fsh",
            "#version 120\n#include \"/lib/fault.glsl\"\nvoid main() { gl_FragColor = used(); }\n",
        ),
        (
            "shaders/lib/fault.glsl",
            "vec2 unused() {\n    return gl_MultiTexCoord1.xy;\n}\nvec4 used() {\n    return vec4(missing_colour);\n}\n",
        ),
    ]);
    let macros = Macros::standard();
    let errors = translate_composite(
        &preprocess(&pack, "composite.vsh", &macros),
        &preprocess(&pack, "composite.fsh", &macros),
        Target::Window,
    )
    .err()
    .expect("the live error must remain");
    assert!(
        errors.iter().any(|e| e.file == "lib/fault.glsl"
            && e.line == 5
            && e.message.contains("missing_colour")),
        "{errors:?}"
    );
}

#[test]
fn keeps_texture_overloads_reached_through_legacy_texture_calls() {
    let program = fragment(
        r#"#version 120
const bvec3 noisetex = bvec3(true);
vec4 hz_n2_img(vec2 p) { return vec4(p, 0.25, 1.0); }
vec4 texture(bvec3 t, vec2 p) { return hz_n2_img(p); }
vec4 textureLod(bvec3 t, vec2 p, float lod) { return hz_n2_img(p + lod); }
vec4 textureGrad(bvec3 t, vec2 p, vec2 dx, vec2 dy) { return hz_n2_img(p + dx + dy); }
vec4 textureProj(bvec3 t, vec4 p) { return hz_n2_img(p.xy / p.w); }
vec4 texelFetch(bvec3 t, ivec2 c, int lod) { return hz_n2_img(vec2(c) / float(1 + lod)); }
void main() {
    vec2 uv = gl_FragCoord.xy;
    vec4 a = texture2D(noisetex, uv);
    vec4 b = texture2DLod(noisetex, uv, 0.0);
    vec4 c = texture2DGrad(noisetex, uv, vec2(0.01), vec2(0.01));
    vec4 d = texture2DProj(noisetex, vec4(uv, 0.0, 1.0));
    vec4 e = texelFetch2D(noisetex, ivec2(1, 2), 0);
    gl_FragColor = a + b + c + d + e;
}
"#,
    )
    .unwrap_or_else(|errors| panic!("legacy calls must retain bvec3 overloads: {errors:?}"));
    assert!(
        program
            .fragment
            .functions
            .iter()
            .any(|(_, f)| f.name.as_deref() == Some("hz_n2_img")),
        "helper reached through legacy texture2D call should remain in fragment module"
    );
}
