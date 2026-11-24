//! Naga's SPIR-V writer gives every loop a counter of its own and one more exit
//! (`force_loop_bounding`, which wgpu asks for), so that no driver meets an endless loop. That
//! also hides the trip count of `for (int i = 0; i < 9; i++)` from the driver's compiler. A loop
//! naga proves finite (`naga::back::bounded_loop`) gets no such bound; every other loop keeps it.

use horizon_shaders::{Macros, Pack, Target, preprocess, translate_composite};

/// How many loops of `module` keep the forced bound in the SPIR-V wgpu would hand the driver:
/// with debug names on, the writer names each bound's counter `loop_bound`.
fn bounds(module: &naga::Module) -> usize {
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(module)
    .unwrap_or_else(|e| panic!("{e:?}"));
    let entry = &module.entry_points[0];
    let pipeline = naga::back::spv::PipelineOptions {
        shader_stage: entry.stage,
        entry_point: entry.name.clone(),
    };
    let options = naga::back::spv::Options {
        flags: naga::back::spv::WriterFlags::DEBUG,
        force_loop_bounding: true,
        ..Default::default()
    };
    let words = naga::back::spv::write_vec(module, &info, &options, Some(&pipeline))
        .unwrap_or_else(|e| panic!("{e:?}"));
    let bytes: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
    let name: &[u8] = b"loop_bound";
    bytes
        .windows(name.len())
        .filter(|window| *window == name)
        .count()
}

/// [`bounds`] of a GLSL fragment shader whose `main` is `body`, after `helpers`.
fn glsl(helpers: &str, body: &str) -> usize {
    let source = format!(
        "#version 450\nlayout(location = 0) out vec4 color;\n{helpers}\nvoid main() {{\n{body}\n}}\n"
    );
    let module = naga::front::glsl::Frontend::default()
        .parse(
            &naga::front::glsl::Options::from(naga::ShaderStage::Fragment),
            &source,
        )
        .unwrap_or_else(|e| panic!("{body}\n{e:?}"));
    bounds(&module)
}

#[test]
fn counted_loops_get_no_forced_bound() {
    for body in [
        // A count up to a literal, and down to one.
        "float a = 0.0;\nfor (int i = 0; i < 9; i++) { a += float(i); }\ncolor = vec4(a);",
        "float a = 0.0;\nfor (int i = 7; i > 0; i--) { a += 1.0; }\ncolor = vec4(a);",
        "float a = 0.0;\nfor (uint i = 0u; i < 8u; i++) { a += 1.0; }\ncolor = vec4(a);",
        // A step of one below a bound only known as the loop runs: `i` can't pass the end of
        // its type, where no bound lets it through.
        "int n = int(gl_FragCoord.x);\nfloat a = 0.0;\n\
         for (int i = 0; i < n; i++) { a += 1.0; }\ncolor = vec4(a);",
        // `<=` and a step of two, against a bound known to stay away from the end of the type:
        // a literal, and a variable only ever set to one.
        "float a = 0.0;\nfor (int i = -3; i <= 3; i++) { a += 1.0; }\ncolor = vec4(a);",
        "float a = 0.0;\nfor (int i = 0; i < 10; i += 2) { a += 1.0; }\ncolor = vec4(a);",
        "int samples = 4;\nfloat a = 0.0;\n\
         for (int i = 1; i <= samples; i++) { a += 1.0; }\ncolor = vec4(a);",
        // A `continue` still reaches the step of a `for`.
        "float a = 0.0;\nfor (int i = 0; i < 9; i++) {\n\
         if (gl_FragCoord.y > float(i)) continue;\na += 1.0;\n}\ncolor = vec4(a);",
        // One operand of `&&` bounds the loop.
        "float a = 0.0;\nfor (int i = 0; i < 9 && a < 4.0; i++) { a += 1.0; }\ncolor = vec4(a);",
        // Both loops of a nest.
        "float a = 0.0;\nfor (int i = 0; i < 4; i++) {\n\
         for (int j = 0; j < 2; j++) { a += 1.0; }\n}\ncolor = vec4(a);",
        // The step in the body, which every iteration that goes on runs.
        "float a = 0.0;\nint i = 0;\nwhile (i < 4) { a += 1.0; i++; }\ncolor = vec4(a);",
        "float a = 0.0;\nint i = 0;\ndo { a += 1.0; i++; } while (i < 4);\ncolor = vec4(a);",
    ] {
        assert_eq!(glsl("", body), 0, "{body}");
    }
}

#[test]
fn loops_not_proven_finite_keep_their_bound() {
    for body in [
        // A float stops growing by one at 2^24.
        "float a = 0.0;\nfor (float x = 0.0; x < 4.0; x++) { a += x; }\ncolor = vec4(a);",
        // `!=` can be stepped over.
        "float a = 0.0;\nfor (int i = 0; i != 9; i++) { a += 1.0; }\ncolor = vec4(a);",
        // The body sets the counter too.
        "float a = 0.0;\nfor (int i = 0; i < 9; i++) {\n\
         if (gl_FragCoord.y > 4.0) i += 2;\na += 1.0;\n}\ncolor = vec4(a);",
        // `i <= n` never fails for the highest `n`, and a step of two can jump past the
        // highest value and wrap around.
        "int n = int(gl_FragCoord.x);\nfloat a = 0.0;\n\
         for (int i = 0; i <= n; i++) { a += 1.0; }\ncolor = vec4(a);",
        "int n = int(gl_FragCoord.x);\nfloat a = 0.0;\n\
         for (int i = 0; i < n; i += 2) { a += 1.0; }\ncolor = vec4(a);",
        // An unsigned counter is never below zero.
        "float a = 0.0;\nfor (uint i = 3u; i >= 0u; i--) {\n\
         a += 1.0;\nif (a > 8.0) break;\n}\ncolor = vec4(a);",
        // A `continue` that skips the step, and one that skips the comparison.
        "float a = 0.0;\nint i = 0;\nwhile (i < 4) {\n\
         if (gl_FragCoord.y > float(i)) { a += 1.0; continue; }\ni++;\n}\ncolor = vec4(a);",
        "float a = 0.0;\nint i = 0;\ndo {\ni++;\n\
         if (gl_FragCoord.y > float(i)) continue;\na += 1.0;\n} while (i < 4);\ncolor = vec4(a);",
    ] {
        assert_eq!(glsl("", body), 1, "{body}");
    }
    // A function handed the counter may set it.
    let body = "float a = 0.0;\nfor (int i = 0; i < 9; i++) { bump(i); a += 1.0; }\n\
                color = vec4(a);";
    assert_eq!(glsl("void bump(inout int k) { k -= 1; }", body), 1);
}

/// Horizon's own shaders are WGSL, whose `for` leaves the loop from the `else` of its test.
#[test]
fn a_counted_wgsl_loop_gets_no_forced_bound() {
    let counted = "@fragment\nfn main() -> @location(0) vec4<f32> {\n    var a = 0.0;\n    \
                   for (var i = 0; i < 9; i++) { a += 1.0; }\n    return vec4<f32>(a);\n}\n";
    assert_eq!(bounds(&naga::front::wgsl::parse_str(counted).unwrap()), 0);
    let endless = "@fragment\nfn main() -> @location(0) vec4<f32> {\n    var a = 0.0;\n    \
                   loop { a += 1.0; if a > 8.0 { break; } }\n    return vec4<f32>(a);\n}\n";
    assert_eq!(bounds(&naga::front::wgsl::parse_str(endless).unwrap()), 1);
}

/// A pack's filter loop over a table of offsets, as its translated program reaches SPIR-V.
#[test]
fn a_pack_programs_counted_loop_gets_no_forced_bound() {
    let vertex = "#version 120\nvarying vec2 texcoord;\nvoid main() {\n    \
                  gl_Position = ftransform();\n    \
                  texcoord = (gl_TextureMatrix[0] * gl_MultiTexCoord0).xy;\n}\n";
    let fragment = "#version 120\nuniform sampler2D colortex0;\nvarying vec2 texcoord;\n\
                    vec2 offsets[4] = vec2[4](vec2(0.0, 0.0), vec2(1.0, 0.0),\n    \
                    vec2(0.0, 1.0), vec2(1.0, 1.0));\n\
                    void main() {\n    vec4 sum = vec4(0.0);\n    \
                    for (int i = 0; i < 4; i++) {\n        \
                    sum += texture2D(colortex0, texcoord + offsets[i] / 64.0);\n    }\n    \
                    gl_FragData[0] = sum / 4.0;\n}\n";
    let zip = {
        use std::io::Write;
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (path, source) in [
            ("shaders/composite.vsh", vertex),
            ("shaders/composite.fsh", fragment),
        ] {
            zip.start_file(path, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(source.as_bytes()).unwrap();
        }
        zip.finish().unwrap().into_inner()
    };
    let pack = Pack::from_zip_bytes("counted-loop.zip", zip).unwrap();
    let macros = Macros::standard();
    let program = translate_composite(
        &preprocess(&pack, "composite.vsh", &macros),
        &preprocess(&pack, "composite.fsh", &macros),
        Target::Buffers,
    )
    .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(bounds(&program.fragment), 0);
}
