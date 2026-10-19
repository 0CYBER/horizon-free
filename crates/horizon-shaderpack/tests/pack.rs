//! The preprocessing half of the Java shader pack translation on a small
//! hand-written pack (`tests/fixtures/mini`), and on real packs when
//! `HORIZON_TEST_SHADERPACKS` names a folder of them (development only:
//! third-party packs are never committed).

use horizon_shaderpack::{
    JavaPack, Stage,
    pipeline::{TargetSettings, TargetSize},
    preprocess::Environment,
};
use std::{
    collections::BTreeMap,
    io::Write,
    path::{Path, PathBuf},
};

fn mini() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mini")
}

fn env() -> Environment {
    Environment::iris("Apple", "Apple M4 Pro", 4)
}

fn saved(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn finds_programs_and_options() {
    let pack = JavaPack::open(&mini()).unwrap();
    assert_eq!(
        pack.programs,
        ["composite.fsh", "composite.vsh", "world-1/composite.fsh"]
    );
    assert_eq!(Stage::of(&pack.programs[1]), Some(Stage::Vertex));
    let bools: Vec<(&str, bool)> = pack
        .options
        .bools
        .values()
        .map(|o| (o.name.as_str(), o.default))
        .collect();
    assert_eq!(
        bools,
        [("BLOOM", true), ("DOF", false)],
        "UNREFERENCED has no #ifdef"
    );
    let strings: Vec<(&str, &str)> = pack
        .options
        .strings
        .values()
        .map(|o| (o.name.as_str(), o.default.as_str()))
        .collect();
    assert_eq!(strings, [("QUALITY", "2"), ("sunPathRotation", "-40.0")]);
    assert_eq!(
        pack.options.strings["QUALITY"].comment.as_deref(),
        Some("Quality")
    );
}

#[test]
fn preprocesses_with_defaults_and_with_values() {
    let pack = JavaPack::open(&mini()).unwrap();
    let defaults = pack
        .preprocess("composite.fsh", &pack.values(&BTreeMap::new()), &env())
        .unwrap();
    assert_eq!(defaults.version, ["120"]);
    assert_eq!(
        defaults.extensions,
        [["GL_ARB_shader_texture_lod", ":", "enable"]]
    );
    let text = defaults
        .text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    assert!(text.contains("float bloomStrength = 0.5 * 2 ;"), "{text}");
    assert!(!text.contains("dofStrength"), "{text}");
    assert!(
        text.contains("float ( 8 )"),
        "Iris's render stage macros: {text}"
    );
    assert!(!text.contains('#'), "no directive is left: {text}");

    let values = pack.values(&saved(&[
        ("BLOOM", "false"),
        ("DOF", "true"),
        ("QUALITY", "3"),
    ]));
    let changed = pack
        .preprocess("composite.fsh", &values, &env())
        .unwrap()
        .text;
    assert!(
        !changed.contains("bloomStrength") && changed.contains("dofStrength"),
        "{changed}"
    );

    let mut source = pack.source("composite.fsh", &values).unwrap();
    assert!(
        source
            .text
            .contains("#define QUALITY 3 // OptionAnnotatedSource: Changed option")
    );
    // Output lines are the inlined lines: errors can name file:line.
    let line = defaults
        .text
        .lines()
        .position(|l| l.contains("bloomStrength"))
        .unwrap() as u32
        + 1;
    assert_eq!(source.locate(line), "lib/common.glsl:3");
    source.text = "#if\n".into();
    assert!(
        horizon_shaderpack::preprocess::preprocess(&source, &env())
            .unwrap_err()
            .starts_with("composite.fsh:1:")
    );
}

#[test]
fn reports_missing_includes_and_cycles() {
    let pack = JavaPack::open(&mini()).unwrap();
    let err = pack
        .source("world-1/composite.fsh", &Default::default())
        .err()
        .unwrap();
    assert_eq!(
        err,
        "world-1/composite.fsh:3: #include \"lib/missing.glsl\": file not found"
    );

    let dir = scratch("cycle");
    std::fs::create_dir_all(dir.join("shaders/lib")).unwrap();
    std::fs::write(
        dir.join("shaders/final.fsh"),
        "#include \"/lib/a.glsl\"\nvoid main() {}\n",
    )
    .unwrap();
    std::fs::write(dir.join("shaders/lib/a.glsl"), "#include \"b.glsl\"\n").unwrap();
    std::fs::write(dir.join("shaders/lib/b.glsl"), "#include \"a.glsl\"\n").unwrap();
    let pack = JavaPack::open(&dir).unwrap();
    let err = pack.source("final.fsh", &Default::default()).err().unwrap();
    assert_eq!(
        err,
        "#include cycle: final.fsh -> lib/a.glsl -> lib/b.glsl -> lib/a.glsl"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn opens_zips_with_shaders_in_a_subfolder() {
    let dir = scratch("zip");
    std::fs::create_dir_all(&dir).unwrap();
    let zip_path = dir.join("Mini-main.zip");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
    let options = zip::write::SimpleFileOptions::default();
    let root = mini().join("shaders");
    for rel in [
        "composite.fsh",
        "composite.vsh",
        "lib/common.glsl",
        "lib/settings.glsl",
        "world-1/composite.fsh",
    ] {
        zip.start_file(format!("Mini-main/shaders/{rel}"), options)
            .unwrap();
        zip.write_all(&std::fs::read(root.join(rel)).unwrap())
            .unwrap();
    }
    zip.start_file("Mini-main/docs/shaders/readme.txt", options)
        .unwrap();
    zip.finish().unwrap();
    let zipped = JavaPack::open(&zip_path).unwrap();
    let folder = JavaPack::open(&mini()).unwrap();
    assert_eq!(zipped.programs, folder.programs);
    let values = zipped.values(&BTreeMap::new());
    assert_eq!(
        zipped
            .preprocess("composite.fsh", &values, &env())
            .unwrap()
            .text,
        folder
            .preprocess("composite.fsh", &values, &env())
            .unwrap()
            .text
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn plans_the_full_screen_passes() {
    let pack = JavaPack::open(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/passes"))
        .unwrap();
    let values = pack.values(&BTreeMap::new());
    let properties = pack.properties(&values, &env()).unwrap();
    let passes = pack.passes("", &values, &env(), &properties).unwrap();

    // Settings from the active comments only: colortex1's R8 is #ifdef'd out.
    assert_eq!(passes.targets[1].format, "RGBA16F");
    assert!(!passes.targets[2].clear);
    assert_eq!(passes.targets[3].clear_color, Some([1.0, 0.0, 0.0, 1.0]));
    assert_eq!(passes.targets[3].size, Some(TargetSize::Relative(0.5, 0.5)));
    assert_eq!(passes.targets[4], TargetSettings::default());

    let summary = |p: &horizon_shaderpack::pipeline::Pass| {
        (
            p.program.clone(),
            p.draw_buffers.clone(),
            p.reads_alt.iter().copied().collect::<Vec<_>>(),
        )
    };
    assert_eq!(
        passes.deferred.iter().map(summary).collect::<Vec<_>>(),
        [("deferred".into(), Some(vec![3]), vec![])]
    );
    // composite2 is off; RENDERTARGETS in `#if 0` doesn't count; each pass
    // reads the side its predecessors wrote, and composite1 flips colortex2.
    assert_eq!(
        passes.composite.iter().map(summary).collect::<Vec<_>>(),
        [
            ("composite".into(), Some(vec![0, 1]), vec![3]),
            ("composite1".into(), Some(vec![0]), vec![0, 1, 3]),
        ]
    );
    assert_eq!(passes.composite[0].mipmapped, [3].into());
    assert_eq!(passes.composite[1].scale, 0.5);
    assert_eq!(passes.flipped_after_prepare, [].into());
    assert_eq!(passes.flipped_after_translucent, [3].into());
    assert_eq!(passes.flipped_at_end, [1, 2, 3].into());
    let final_pass = passes.final_pass.unwrap();
    assert_eq!(
        (final_pass.program.as_str(), final_pass.draw_buffers),
        ("final", None)
    );
    assert_eq!(final_pass.reads_alt, [1, 2, 3].into());
}

fn scratch(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("horizon-shaderpack-{name}-{}", std::process::id()))
}

/// Every program of the packs in `HORIZON_TEST_SHADERPACKS` (folders or
/// zips) preprocesses. Skipped when the variable isn't set.
#[test]
fn real_packs_preprocess() {
    let Some(dir) = std::env::var_os("HORIZON_TEST_SHADERPACKS") else {
        return;
    };
    let mut failures = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let Ok(pack) = JavaPack::open(&entry.path()) else {
            continue;
        };
        let values = pack.values(&BTreeMap::new());
        let mut ok = 0;
        for program in &pack.programs {
            match pack.preprocess(program, &values, &env()) {
                Ok(_) => ok += 1,
                Err(e) => failures.push(format!("{}: {program}: {e}", entry.path().display())),
            }
        }
        println!(
            "{}: {ok}/{} programs, {} boolean and {} value options",
            entry.file_name().to_string_lossy(),
            pack.programs.len(),
            pack.options.bools.len(),
            pack.options.strings.len()
        );
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The `shaders.properties` of the packs in `HORIZON_TEST_SHADERPACKS` read
/// with their defaults. Skipped when the variable isn't set.
#[test]
fn real_packs_properties() {
    let Some(dir) = std::env::var_os("HORIZON_TEST_SHADERPACKS") else {
        return;
    };
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let Ok(pack) = JavaPack::open(&entry.path()) else {
            continue;
        };
        let values = pack.values(&BTreeMap::new());
        let properties = pack.properties(&values, &env()).unwrap();
        let profiles = properties.profiles(&pack.options).unwrap();
        let current = horizon_shaderpack::properties::ShaderProperties::current_profile(
            &profiles,
            &pack.options,
            &values,
        );
        let disabled: std::collections::BTreeSet<&str> = pack
            .programs
            .iter()
            .map(|p| horizon_shaderpack::program_name(p))
            .filter(|p| !properties.program_enabled(p, &pack.options, &values))
            .collect();
        let main = properties.screen(None).unwrap();
        println!(
            "{}: {} main screen elements, {} sub-screens, {} sliders, profiles {:?} (current {:?}), off: {}",
            entry.file_name().to_string_lossy(),
            main.len(),
            properties.screens().len(),
            properties.sliders().len(),
            profiles.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            current.map(|p| p.name.as_str()),
            disabled.into_iter().collect::<Vec<_>>().join(" ")
        );
        // Screens name the pack's options (a few stale names are tolerated:
        // Iris shows them as empty cells).
        let mut unknown = Vec::new();
        for screen in std::iter::once(None).chain(properties.screens().into_iter().map(Some)) {
            for e in properties.screen(screen).unwrap() {
                if let horizon_shaderpack::properties::MenuElement::Option(o) = e {
                    if !pack.options.bools.contains_key(&o)
                        && !pack.options.strings.contains_key(&o)
                    {
                        unknown.push(o);
                    }
                }
            }
        }
        println!("  unknown screen options: {unknown:?}");
        assert!(unknown.len() <= 2, "{unknown:?}");
        // Every custom uniform parses.
        let uniforms = properties.custom_uniforms();
        println!(
            "  custom uniforms: {:?}, sky {:?}",
            uniforms.problems,
            properties.sky()
        );
        assert!(uniforms.problems.is_empty(), "{:?}", uniforms.problems);
    }
}

/// The overworld's full-screen passes for the packs in
/// `HORIZON_TEST_SHADERPACKS`. Skipped when the variable isn't set.
#[test]
fn real_packs_passes() {
    let Some(dir) = std::env::var_os("HORIZON_TEST_SHADERPACKS") else {
        return;
    };
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let Ok(pack) = JavaPack::open(&entry.path()) else {
            continue;
        };
        let values = pack.values(&BTreeMap::new());
        let properties = pack.properties(&values, &env()).unwrap();
        let folder = if pack.programs.iter().any(|p| p.starts_with("world0/")) {
            "world0"
        } else {
            ""
        };
        let passes = pack.passes(folder, &values, &env(), &properties).unwrap();
        println!("{}:", entry.file_name().to_string_lossy());
        for (i, t) in passes.targets.iter().enumerate() {
            if *t != TargetSettings::default() {
                println!("  colortex{i}: {t:?}");
            }
        }
        for p in passes
            .begin
            .iter()
            .chain(&passes.prepare)
            .chain(&passes.deferred)
            .chain(&passes.composite)
            .chain(&passes.final_pass)
        {
            println!(
                "  {} writes {:?} reads alt {:?} mipmaps {:?} scale {}",
                p.program, p.draw_buffers, p.reads_alt, p.mipmapped, p.scale
            );
        }
        assert!(passes.final_pass.is_some() || !passes.composite.is_empty());
        // Every planned pass translates, its outputs within its targets.
        for p in passes
            .begin
            .iter()
            .chain(&passes.prepare)
            .chain(&passes.deferred)
            .chain(&passes.composite)
            .chain(&passes.final_pass)
        {
            let program = pack
                .compile(p, &passes.targets, &values, &env(), &properties)
                .unwrap();
            assert!(
                program
                    .fragment
                    .interface
                    .outputs
                    .keys()
                    .all(|&l| (l as usize) < program.draw_buffers.len())
                    || p.program.ends_with("final"),
                "{}: outputs {:?}, targets {:?}",
                p.program,
                program.fragment.interface.outputs,
                program.draw_buffers
            );
        }
    }
}
