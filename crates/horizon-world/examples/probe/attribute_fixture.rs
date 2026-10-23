use crate::support::common::f64_bits;
use horizon_world::attributes::{
    AttributeError, Definition, Instance, Modifier, Observer, Operation, Packed,
};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
fn number(v: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
fn refs<'a>(mods: impl IntoIterator<Item = &'a Modifier>, pool: &[Modifier]) -> String {
    let s = mods
        .into_iter()
        .map(|m| {
            pool.iter()
                .position(|p| p.same_identity(m))
                .unwrap()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join(",");
    if s.is_empty() { "-".into() } else { s }
}
struct Callback<'a> {
    out: &'a mut dyn Write,
    target: usize,
    calls: i64,
    op: &'a Value,
}
impl Observer for Callback<'_> {
    type Error = io::Error;
    fn dirty(&mut self, a: &mut Instance) -> io::Result<()> {
        writeln!(
            self.out,
            "dirty {} {:016x} {} {:016x} {} {}",
            self.target,
            f64_bits(a.base_value()),
            a.is_dirty(),
            f64_bits(a.cached_value()),
            a.modifiers().len(),
            a.permanent_modifiers().len()
        )?;
        let n = self.calls;
        self.calls += 1;
        if n == self.op["fail"].as_i64().unwrap() {
            return Err(io::Error::other("injected attribute observer failure"));
        }
        if self.op["read"].as_bool().unwrap() {
            writeln!(self.out, "observed {:016x}", f64_bits(a.value()))?;
        }
        Ok(())
    }
}
fn state(out: &mut dyn Write, id: usize, a: &Instance, pool: &[Modifier]) -> io::Result<()> {
    writeln!(
        out,
        "state {id} {:016x} {} {:016x}",
        f64_bits(a.base_value()),
        a.is_dirty(),
        f64_bits(a.cached_value())
    )?;
    writeln!(out, "all {}", refs(a.modifiers(), pool))?;
    writeln!(out, "permanent {}", refs(a.permanent_modifiers(), pool))?;
    for op in Operation::ALL {
        writeln!(
            out,
            "operation {} {}",
            op as u8,
            refs(a.operation_modifiers(op), pool)
        )?;
    }
    Ok(())
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).expect("private fixture directory");
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let definitions = f["definitions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| {
                let d = number(&v["default"]);
                if v.get("min").is_some() {
                    Definition::ranged(d, number(&v["min"]), number(&v["max"]))
                } else {
                    Ok(Definition::basic(d))
                }
            })
            .collect::<io::Result<Vec<_>>>();
        let Ok(definitions) = definitions else {
            writeln!(out, "definition error")?;
            continue;
        };
        let mut instances = [Instance::new(definitions[0]), Instance::new(definitions[1])];
        let pool = f["modifiers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| {
                Modifier::new(
                    v["id"].as_str().unwrap(),
                    number(&v["amount"]),
                    Operation::ALL[v["operation"].as_u64().unwrap() as usize],
                )
            })
            .collect::<io::Result<Vec<_>>>()?;
        let mut packed: Option<Packed> = None;
        for op in f["ops"].as_array().unwrap() {
            let target = op["target"].as_u64().unwrap() as usize;
            let mut cb = Callback {
                out: &mut out,
                target,
                calls: 0,
                op,
            };
            let m = || pool[op["modifier"].as_u64().unwrap() as usize].clone();
            let mods = || {
                op["modifiers"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| pool[v.as_u64().unwrap() as usize].clone())
                    .collect::<Vec<_>>()
            };
            let result = match op["mode"].as_str().unwrap() {
                "base" => instances[target].set_base_value(number(&op["base"]), &mut cb),
                "add" => instances[target].add_transient(m(), &mut cb),
                "update" => instances[target].add_or_update_transient(m(), &mut cb),
                "permanent" => instances[target].add_permanent(m(), &mut cb),
                "replace" => instances[target].add_or_replace_permanent(m(), &mut cb),
                "remove" => {
                    let v = instances[target].remove(m().id(), &mut cb);
                    match v {
                        Ok(v) => {
                            writeln!(cb.out, "removed {v}")?;
                            Ok(())
                        }
                        Err(e) => Err(e),
                    }
                }
                "clear" => instances[target].remove_all(&mut cb),
                "copy" => {
                    let source = op["source"].as_u64().unwrap() as usize;
                    if source == target {
                        instances[target].replace_from_self(&mut cb)
                    } else {
                        let snapshot = instances[source].clone();
                        instances[target].replace_from(&snapshot, &mut cb)
                    }
                }
                "pack" => {
                    let p = instances[target].pack();
                    writeln!(
                        cb.out,
                        "packed {:016x} {}",
                        f64_bits(p.base),
                        refs(&p.modifiers, &pool)
                    )?;
                    packed = Some(p);
                    Ok(())
                }
                "unpack" => instances[target].apply(packed.as_ref().unwrap(), &mut cb),
                "apply" => instances[target].apply(
                    &Packed {
                        base: number(&op["base"]),
                        modifiers: mods(),
                    },
                    &mut cb,
                ),
                "many" => instances[target].add_permanent_many(mods(), &mut cb),
                "value" => {
                    writeln!(cb.out, "value {:016x}", f64_bits(instances[target].value()))?;
                    Ok(())
                }
                _ => panic!("fixture mode"),
            };
            match result {
                Ok(()) => writeln!(cb.out, "result ok")?,
                Err(AttributeError::DuplicateModifier(_)) => writeln!(cb.out, "error duplicate")?,
                Err(AttributeError::Observer(_)) => writeln!(cb.out, "error observer")?,
            };
            state(cb.out, 0, &instances[0], &pool)?;
            state(cb.out, 1, &instances[1], &pool)?;
            writeln!(cb.out, "calls {}", cb.calls)?;
        }
    }
    Ok(())
}
