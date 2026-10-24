use horizon_core::{
    position::BlockPos,
    random::{
        LegacyRandom, PositionalRandom, RandomSource, WorldgenRandom, WorldgenSource,
        XoroshiroRandom, slime_chunk_random,
    },
};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
fn n(v: &Value) -> i32 {
    v.as_i64().unwrap() as i32
}
fn l(v: &Value) -> i64 {
    v.as_i64().unwrap()
}
fn fixture(f: &Value, source: impl WorldgenSource, out: &mut impl Write) -> io::Result<()> {
    let mut root = WorldgenRandom::new(source);
    let mut child = None;
    for op in f["ops"].as_array().unwrap() {
        match op[0].as_str().unwrap() {
            "root" => {
                child = None;
                writeln!(out, "root")?;
            }
            "bits" => writeln!(out, "bits {}", root.next_bits(n(&op[1])))?,
            "decoration" => writeln!(
                out,
                "decoration {}",
                root.set_decoration_seed(l(&op[1]), n(&op[2]), n(&op[3]))
            )?,
            "feature" => {
                root.set_feature_seed(l(&op[1]), n(&op[2]), n(&op[3]));
                writeln!(out, "feature")?;
            }
            "large" => {
                root.set_large_feature_seed(l(&op[1]), n(&op[2]), n(&op[3]));
                writeln!(out, "large")?;
            }
            "salt" => {
                root.set_large_feature_with_salt(l(&op[1]), n(&op[2]), n(&op[3]), n(&op[4]));
                writeln!(out, "salt")?;
            }
            "slime" => {
                let mut r = slime_chunk_random(n(&op[1]), n(&op[2]), l(&op[3]), l(&op[4]));
                writeln!(
                    out,
                    "slime {} {}",
                    r.next_long(),
                    r.next_int_bound(10).unwrap()
                )?;
            }
            kind => {
                let random = child.as_mut().unwrap_or(&mut root);
                match kind {
                    "int" => writeln!(out, "int {}", random.next_int())?,
                    "long" => writeln!(out, "long {}", random.next_long())?,
                    "bool" => writeln!(out, "bool {}", random.next_boolean())?,
                    "float" => writeln!(out, "float {:x}", random.next_float().to_bits())?,
                    "double" => writeln!(out, "double {:x}", random.next_double().to_bits())?,
                    "bound" => match random.next_int_bound(n(&op[1])) {
                        Ok(v) => writeln!(out, "bound {v}")?,
                        Err(_) => writeln!(out, "bound error")?,
                    },
                    "seed" => {
                        random.set_seed(l(&op[1]));
                        writeln!(out, "seed")?;
                    }
                    "consume" => {
                        random.consume_count(n(&op[1]));
                        writeln!(out, "consume")?;
                    }
                    "fork" => {
                        child = Some(random.fork());
                        writeln!(out, "fork")?;
                    }
                    "pos" => {
                        child = Some(random.fork_positional().at(BlockPos::new(
                            n(&op[1]),
                            n(&op[2]),
                            n(&op[3]),
                        )));
                        writeln!(out, "pos")?;
                    }
                    "pos_seed" => {
                        child = Some(random.fork_positional().from_seed(l(&op[1])));
                        writeln!(out, "pos_seed")?;
                    }
                    "pos_hash" => {
                        let units = op[1]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|v| n(v) as u16)
                            .collect::<Vec<_>>();
                        child = Some(random.fork_positional().from_hash_of_utf16(&units));
                        writeln!(out, "pos_hash")?;
                    }
                    _ => panic!("unknown operation"),
                }
            }
        }
        writeln!(out, "count {}", root.count())?;
    }
    writeln!(out, "tail {}", root.next_long())?;
    writeln!(out, "count {}", root.count())?;
    Ok(())
}
fn layers(f: &Value, mut source: impl WorldgenSource, out: &mut impl Write) -> io::Result<()> {
    source.consume_count(n(&f["warmup"]));
    match n(&f["layers"]) {
        1 => fixture(f, source, out),
        2 => fixture(f, WorldgenRandom::new(source), out),
        3 => fixture(f, WorldgenRandom::new(WorldgenRandom::new(source)), out),
        _ => panic!("unsupported layer count"),
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).expect("private fixture directory");
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        if f["legacy"].as_bool().unwrap() {
            layers(&f, LegacyRandom::new(l(&f["seed"])), &mut out)?;
        } else {
            layers(&f, XoroshiroRandom::new(l(&f["seed"])), &mut out)?;
        }
    }
    Ok(())
}
