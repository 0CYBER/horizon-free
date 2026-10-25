//! Reads real packs as black boxes: `HORIZON_TEST_SHADER_PACKS=<folder> cargo test -p
//! horizon-shaders --release -- --ignored --nocapture`. The folder (and its subfolders) holds
//! pack zips or folders, which are never committed; this prints what was read and how long it took.

use std::path::Path;
use std::time::Instant;

use horizon_shaders::{Lang, Pack, discover};

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
        let properties = pack.read_text("shaders.properties");
        let lang = Lang::load(&pack, "en_us");
        println!(
            "{}: {} files at the root, shaders.properties {} bytes, lang {}, {:.2} ms",
            f.name,
            pack.list("").len(),
            properties.as_ref().map_or(0, String::len),
            if lang == Lang::default() {
                "none"
            } else {
                "read"
            },
            start.elapsed().as_secs_f64() * 1000.0
        );
        assert!(properties.is_some(), "{}: no shaders.properties", f.name);
        read += 1;
    }
    assert!(read > 0, "no packs under {}", dir.display());
}
