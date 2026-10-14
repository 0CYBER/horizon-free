//! Compare the exact read/deserialize sequences used by the model loaders.
//! Run with `cargo run -p horizon-client --profile fast --example json_readers_bench`.
//! Optional arguments: resource directory, repetitions (default 9).
use serde_json::{Map, Value, json};
use std::{
    collections::BTreeMap,
    fmt::Debug,
    fs::{self, File},
    hint::black_box,
    io::BufReader,
    path::{Path, PathBuf},
    time::Instant,
};

type Result<T> = horizon_client::Result<T>;

#[derive(Clone, Copy)]
enum Reader {
    File,
    Buffered,
    Slice,
}
const READERS: [Reader; 3] = [Reader::File, Reader::Buffered, Reader::Slice];

// These match the map types and reader expressions at all three production sites.
macro_rules! reader {
    ($name:ident, $map:ty) => {
        fn $name(path: &Path, reader: Reader) -> Result<$map> {
            Ok(match reader {
                Reader::File => serde_json::from_reader(File::open(path)?)?,
                Reader::Buffered => serde_json::from_reader(BufReader::new(File::open(path)?))?,
                Reader::Slice => serde_json::from_slice(&fs::read(path)?)?,
            })
        }
    };
}
reader!(prepare_facts, Map<String, Value>);
reader!(catalog_facts, BTreeMap<String, Value>);

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Result<Self> {
        let root =
            std::env::temp_dir().join(format!("horizon-json-readers-{}", std::process::id()));
        fs::create_dir(&root)?;
        Ok(Self(root))
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn fixtures(root: &Path) -> Result<()> {
    let states: Vec<_> = (0..56)
        .map(|i| {
            json!({"facing": (["north", "east", "south", "west"][i % 4]),
                "powered": if i / 4 % 2 == 0 { "false" } else { "true" },
                "age": (i / 8).to_string()})
        })
        .collect();
    let mut models = Map::new();
    let mut render = Map::new();
    for i in 0..1024 {
        let name = format!("minecraft:fixture_block_{i:04}");
        models.insert(
            name.clone(),
            json!({
                "schema": {"facing": {"named": ["north", "east", "south", "west"]},
                    "powered": {"named": ["false", "true"]}, "age": {"integer": [0, 6]}},
                "states": states
            }),
        );
        render.insert(
            name,
            json!({"rule": "never", "can_occlude": true, "bars_tag": false}),
        );
    }
    fs::write(root.join("model-states.json"), serde_json::to_vec(&models)?)?;
    fs::write(
        root.join("render-definitions.json"),
        serde_json::to_vec(&render)?,
    )?;
    Ok(())
}

fn benchmark<T: PartialEq + Debug>(
    label: &str,
    path: &Path,
    repetitions: usize,
    read: impl Fn(&Path, Reader) -> Result<T>,
) -> Result<()> {
    let expected = read(path, Reader::File)?;
    for reader in READERS {
        assert_eq!(expected, read(path, reader)?);
        for _ in 0..2 {
            black_box(read(black_box(path), reader)?);
        }
    }
    drop(expected);
    let mut samples: [Vec<f64>; 3] = std::array::from_fn(|_| Vec::new());
    for repetition in 0..repetitions {
        // Rotate reader order to distribute drift across implementations.
        for offset in 0..3 {
            let index = (repetition + offset) % 3;
            let start = Instant::now();
            let value = black_box(read(black_box(path), READERS[index])?);
            samples[index].push(start.elapsed().as_secs_f64() * 1000.0);
            drop(value); // Match the loaders: retain the parsed tree after loading.
        }
    }
    let median: Vec<_> = samples
        .iter_mut()
        .map(|s| {
            s.sort_by(f64::total_cmp);
            s[s.len() / 2]
        })
        .collect();
    println!(
        "{label}: {} bytes, {repetitions} samples/reader, median ms: File={:.3}, BufReader={:.3}, read+slice={:.3}; slice speedup={:.2}x",
        fs::metadata(path)?.len(),
        median[0],
        median[1],
        median[2],
        median[0] / median[2]
    );
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let fixture = if args.is_empty() {
        Some(Fixture::new()?)
    } else {
        None
    };
    let root = fixture
        .as_ref()
        .map(|f| f.0.clone())
        .unwrap_or_else(|| PathBuf::from(&args[0]));
    if fixture.is_some() {
        fixtures(&root)?;
        println!(
            "Generated representative fixtures: 1024 blocks, 57344 model states; warm filesystem cache."
        );
    }
    let repetitions = args.get(1).map(|s| s.parse()).transpose()?.unwrap_or(9);
    assert!(repetitions > 0, "repetitions must be positive");
    benchmark(
        "prepare model states",
        &root.join("model-states.json"),
        repetitions,
        prepare_facts,
    )?;
    benchmark(
        "catalog model states",
        &root.join("model-states.json"),
        repetitions,
        catalog_facts,
    )?;
    benchmark(
        "render definitions",
        &root.join("render-definitions.json"),
        repetitions,
        catalog_facts,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reader_equivalence_and_errors() -> Result<()> {
        let fixture = Fixture::new()?;
        let path = fixture.0.join("test.json");
        fs::write(&path, br#"{"minecraft:stone":{"schema":{},"states":[{}]},"unicode":"\u2603","duplicate":1,"duplicate":2}"#)?;
        let expected = prepare_facts(&path, Reader::File)?;
        for reader in READERS {
            assert_eq!(expected, prepare_facts(&path, reader)?);
            assert_eq!(
                expected.iter().collect::<BTreeMap<_, _>>(),
                catalog_facts(&path, reader)?.iter().collect()
            );
        }
        for invalid in [b"{\"a\": ".as_slice(), b"{} trailing", b"{\"a\":\"\xff\"}"] {
            fs::write(&path, invalid)?;
            for reader in READERS {
                let error = prepare_facts(&path, reader).unwrap_err();
                assert!(error.downcast_ref::<serde_json::Error>().is_some());
                assert!(catalog_facts(&path, reader).is_err());
            }
        }
        fs::remove_file(&path)?;
        for reader in READERS {
            let error = prepare_facts(&path, reader).unwrap_err();
            assert_eq!(
                error.downcast_ref::<std::io::Error>().unwrap().kind(),
                std::io::ErrorKind::NotFound
            );
        }
        Ok(())
    }
}
