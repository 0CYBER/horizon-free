//! `flip.<program>.<buffer> = true` on a buffer the program writes (#9010). The expected values
//! are Iris 1.11.7's, measured with probe packs of our own.
use super::destination_tests::Fixture;
use std::io::{Cursor, Write};

const VERTEX: &str = "#version 120\nvarying vec2 uv;\nvoid main() { uv=gl_MultiTexCoord0.xy; gl_Position=ftransform(); }\n";

/// `composite` writes 0.25 to colortex2 and colortex4 and forces both to flip: the forced flip
/// cancels the write's own, so `composite1` samples colortex2's cleared side (Iris 1.11.7: 0),
/// and `composite2`, blended `ONE ONE`, adds 0.5 onto colortex4's 0.25 (Iris 1.11.7: 0.749).
#[test]
fn a_forced_flip_of_a_written_buffer_leaves_it_unflipped() {
    let Some(gpu) = Fixture::new() else { return };
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let mut add = |name: &str, source: &str| {
        zip.start_file(
            format!("shaders/{name}"),
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(source.as_bytes()).unwrap();
    };
    for (program, fragment) in [
        (
            "composite",
            "#version 120\n/* RENDERTARGETS:2,4 */\n\
             void main() { gl_FragData[0]=vec4(0.25,0.25,0.25,1); gl_FragData[1]=vec4(0.25,0.25,0.25,1); }\n",
        ),
        (
            "composite1",
            "#version 120\nuniform sampler2D colortex2;\nvarying vec2 uv;\n/* RENDERTARGETS:3 */\n\
             void main() { gl_FragData[0]=texture2D(colortex2,uv); }\n",
        ),
        (
            "composite2",
            "#version 120\n/* RENDERTARGETS:4 */\nvoid main() { gl_FragData[0]=vec4(0.5,0.5,0.5,0); }\n",
        ),
        (
            "final",
            "#version 120\nuniform sampler2D colortex3;\nuniform sampler2D colortex4;\nvarying vec2 uv;\n\
             void main() { gl_FragColor=vec4(texture2D(colortex4,uv).r,texture2D(colortex3,uv).r,0,1); }\n",
        ),
    ] {
        add(&format!("{program}.vsh"), VERTEX);
        add(&format!("{program}.fsh"), fragment);
    }
    add(
        "shaders.properties",
        "flip.composite.colortex2=true\nflip.composite.colortex4=true\nblend.composite2=ONE ONE ONE ONE\n",
    );
    let pack = horizon_shaders::Pack::from_zip_bytes(
        "forced-flip-of-a-written-buffer",
        zip.finish().unwrap().into_inner(),
    )
    .unwrap();
    let mut macros = horizon_shaders::Macros::standard();
    let (properties, _) = horizon_shaders::read_properties(&pack, &mut macros);
    let post = horizon_shaders::load_post(&pack, "", &properties, &macros, &Default::default());
    let mut post = gpu.build(post);
    for frame in 0..2 {
        let actual = gpu.render(&mut post, [16, 16]);
        let wrong: Vec<_> = (actual.iter().enumerate())
            .filter(|(_, p)| {
                p.iter()
                    .zip([191, 0, 0, 255])
                    .any(|(a, b)| a.abs_diff(b) > 1)
            })
            .map(|(n, p)| (n, *p))
            .take(4)
            .collect();
        assert!(wrong.is_empty(), "frame {frame}: {wrong:?}");
    }
}

/// `composite1` writes no buffer but forces colortex2 to flip. `composite` writes 0.5 there and
/// flips it itself, so the two flips cancel and `final` samples the cleared side. Iris 1.11.7
/// applies the dropped program's flips at that point in the chain (a pack does this with
/// `composite45`); reading the written side means they were lost (6.1.6).
#[test]
fn a_flip_of_a_program_that_writes_nothing_still_applies() {
    let Some(gpu) = Fixture::new() else { return };
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let mut add = |name: &str, source: &str| {
        zip.start_file(
            format!("shaders/{name}"),
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(source.as_bytes()).unwrap();
    };
    for (program, fragment) in [
        (
            "composite",
            "#version 120\n/* RENDERTARGETS:2 */\n\
             void main() { gl_FragData[0]=vec4(0.5,0.5,0.5,1); }\n",
        ),
        ("composite1", "#version 120\nvoid main() { }\n"),
        (
            "final",
            "#version 120\nuniform sampler2D colortex2;\nvarying vec2 uv;\n\
             void main() { gl_FragColor=vec4(texture2D(colortex2,uv).rgb,1.0); }\n",
        ),
    ] {
        add(&format!("{program}.vsh"), VERTEX);
        add(&format!("{program}.fsh"), fragment);
    }
    add("shaders.properties", "flip.composite1.colortex2=true\n");
    let pack = horizon_shaders::Pack::from_zip_bytes(
        "flip-of-a-program-that-writes-nothing",
        zip.finish().unwrap().into_inner(),
    )
    .unwrap();
    let mut macros = horizon_shaders::Macros::standard();
    let (properties, _) = horizon_shaders::read_properties(&pack, &mut macros);
    let post = horizon_shaders::load_post(&pack, "", &properties, &macros, &Default::default());
    let mut post = gpu.try_build(post).expect("the pack runs");
    assert!(
        post.notes
            .iter()
            .any(|n| n.starts_with("composite1: writes no buffer; skipped")),
        "{:?}",
        post.notes
    );
    let actual = gpu.render(&mut post, [16, 16]);
    let wrong: Vec<_> = actual
        .iter()
        .enumerate()
        .filter(|(_, p)| p.iter().zip([0, 0, 0, 255]).any(|(a, b)| a.abs_diff(b) > 1))
        .map(|(n, p)| (n, *p))
        .take(4)
        .collect();
    assert!(wrong.is_empty(), "{wrong:?}");
}

/// A buffer nobody wrote on the alternate side still needs its B texture when a dropped program
/// forces its flip, and the stage's custom texture over it is released before the next program's
/// bindings are made: `composite` only writes colortex0, `composite1` writes nothing and flips
/// colortex2, and `composite2` must read colortex2 itself (not `texture.composite.colortex2`).
#[test]
fn a_carried_flip_counts_as_a_use_and_releases_the_custom_texture() {
    let Some(gpu) = Fixture::new() else { return };
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let mut add = |name: &str, source: &[u8]| {
        zip.start_file(
            format!("shaders/{name}"),
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(source).unwrap();
    };
    for (program, fragment) in [
        (
            "composite",
            "#version 120\nuniform sampler2D colortex2;\nvarying vec2 uv;\n/* RENDERTARGETS:0 */\n\
             void main() { gl_FragData[0]=texture2D(colortex2,uv); }\n",
        ),
        ("composite1", "#version 120\nvoid main() { }\n"),
        (
            "composite2",
            "#version 120\nuniform sampler2D colortex2;\nvarying vec2 uv;\n/* RENDERTARGETS:3 */\n\
             void main() { gl_FragData[0]=texture2D(colortex2,uv); }\n",
        ),
        (
            "final",
            "#version 120\nuniform sampler2D colortex3;\nvarying vec2 uv;\n\
             void main() { gl_FragColor=vec4(texture2D(colortex3,uv).rgb,1.0); }\n",
        ),
    ] {
        add(&format!("{program}.vsh"), VERTEX.as_bytes());
        add(&format!("{program}.fsh"), fragment.as_bytes());
    }
    add(
        "shaders.properties",
        b"texture.composite.colortex2=d.dat TEXTURE_2D RGBA8 1 1 RGBA UNSIGNED_BYTE\n\
          flip.composite1.colortex2=true\n",
    );
    add("d.dat", &[255, 0, 0, 255]);
    let pack = horizon_shaders::Pack::from_zip_bytes(
        "carried-flip-of-an-unwritten-buffer",
        zip.finish().unwrap().into_inner(),
    )
    .unwrap();
    let mut macros = horizon_shaders::Macros::standard();
    let (properties, _) = horizon_shaders::read_properties(&pack, &mut macros);
    let post = horizon_shaders::load_post(&pack, "", &properties, &macros, &Default::default());
    let post = gpu.try_build(post).expect("the pack runs");
    assert!(
        post.notes
            .iter()
            .any(|n| n.starts_with("composite1: writes no buffer; skipped")),
        "{:?}",
        post.notes
    );
    assert_ne!(
        post.doubled & 1 << 2,
        0,
        "a buffer only a carried flip touches still gets a B texture"
    );
    let composite2 = post
        .passes
        .iter()
        .find(|p| p.name == "composite2")
        .expect("the pass after the dropped one");
    assert!(
        matches!(composite2.samplers[0].source, super::Source::Color(2)),
        "composite2 reads the buffer, not the released custom texture"
    );
}
