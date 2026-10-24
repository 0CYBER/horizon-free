//! Reads real packs as black boxes: `HORIZON_TEST_SHADER_PACKS=<folder> cargo test -p
//! horizon-shaders --release -- --ignored --nocapture`. The folder (and its subfolders) holds
//! pack zips or folders, which are never committed; this prints what was read and how long it took.

use std::path::Path;
use std::time::Instant;

use horizon_shaders::{Lang, Macros, Pack, Properties, discover, preprocess};

#[test]
#[ignore = "needs HORIZON_TEST_SHADER_PACKS"]
fn real_packs_read() {
    let dir = std::env::var_os("HORIZON_TEST_SHADER_PACKS").expect("HORIZON_TEST_SHADER_PACKS");
    let dir = Path::new(&dir);
    let mut found = discover(dir);
    for sub in std::fs::read_dir(dir).unwrap().flatten() {
        if sub.path().is_dir() {
            found.extend(discover(&sub.path()));
        }
    }
    let mut read = 0;
    for f in found.iter().filter(|f| f.problem.is_none()) {
        let start = Instant::now();
        let pack = Pack::open(&f.path).unwrap();
        let macros = Macros::standard();
        let shaders = Properties::parse(
            &pack.read_text("shaders.properties").unwrap_or_default(),
            &macros,
        );
        let ids: Vec<Properties> = ["block", "item", "entity", "dimension"]
            .iter()
            .filter_map(|n| pack.read_text(&format!("{n}.properties")))
            .map(|t| Properties::parse(&t, &macros))
            .collect();
        let lang = Lang::load(&pack, "en_us");
        let elapsed = start.elapsed();
        println!(
            "{}: shaders.properties {} keys ({} notes), {} id files {} keys ({} notes), lang {}, {:.2} ms",
            f.name,
            shaders.entries.len(),
            shaders.notes.len(),
            ids.len(),
            ids.iter().map(|p| p.entries.len()).sum::<usize>(),
            ids.iter().map(|p| p.notes.len()).sum::<usize>(),
            if lang == Lang::default() {
                "none"
            } else {
                "read"
            },
            elapsed.as_secs_f64() * 1000.0
        );
        for n in &shaders.notes {
            println!("  shaders.properties:{}: {}", n.line, n.message);
        }
        // Every stage file of the base and dimension folders through the preprocessor.
        let start = Instant::now();
        let (mut stages, mut lines, mut diagnostics) = (0, 0, Vec::new());
        for dir in ["", "world0", "world-1", "world1"] {
            for file in pack.list(dir) {
                if [".vsh", ".fsh", ".gsh", ".csh"]
                    .iter()
                    .any(|e| file.ends_with(e))
                {
                    let path = if dir.is_empty() {
                        file
                    } else {
                        format!("{dir}/{file}")
                    };
                    let p = preprocess(&pack, &path, &macros);
                    stages += 1;
                    lines += p.lines.len();
                    diagnostics.extend(p.diagnostics.iter().map(|d| {
                        let file = &p.files[d.origin.file as usize];
                        format!("{file}:{}: {}", d.origin.line, d.message)
                    }));
                }
            }
        }
        println!(
            "  {stages} stage files preprocessed, {lines} lines out, {} diagnostics, {:.1} ms",
            diagnostics.len(),
            start.elapsed().as_secs_f64() * 1000.0
        );
        diagnostics.sort();
        diagnostics.dedup();
        for d in diagnostics.iter().take(8) {
            println!("    {d}");
        }
        assert!(!shaders.entries.is_empty(), "{}: no keys read", f.name);
        read += 1;
    }
    assert!(read > 0, "no packs under {}", dir.display());
}
