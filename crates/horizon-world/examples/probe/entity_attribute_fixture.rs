use crate::support::common::f64_bits;
use horizon_world::attributes::{
    AttributeError, Modifier, Operation,
    entity::{Catalog, LookupError},
};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
    sync::Arc,
};
fn number(v: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
fn lookup(out: &mut dyn Write, label: &str, result: Result<f64, LookupError>) -> io::Result<()> {
    match result {
        Ok(v) => {
            writeln!(out, "{label} {:016x}", f64_bits(v))?;
            writeln!(out, "ok")
        }
        Err(e) => writeln!(
            out,
            "error {}",
            match e {
                LookupError::MissingAttribute => "attribute",
                LookupError::MissingModifier => "modifier",
                LookupError::InstanceMissingModifier => "instance_modifier",
            }
        ),
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).unwrap();
    let dir = Path::new(&arg);
    let catalog = Arc::new(Catalog::from_json(&serde_json::from_str(
        &fs::read_to_string(dir.join("catalog.json"))?,
    )?)?);
    let ids: Vec<_> = catalog.attributes().map(|(id, _)| id).collect();
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let mut map = catalog.create(f["entity"].as_str().unwrap()).unwrap();
        for op in f["ops"].as_array().unwrap() {
            let id = ids[op["attribute"].as_u64().unwrap() as usize];
            let mode = op["mode"].as_str().unwrap();
            let modifier = Modifier::new(
                op["id"].as_str().unwrap(),
                number(&op["amount"]),
                Operation::ALL[op["operation"].as_u64().unwrap() as usize],
            )?;
            match mode {
                "value" => lookup(&mut out, "value", map.value(id))?,
                "base" => lookup(&mut out, "base", map.base_value(id))?,
                "modifier" => lookup(&mut out, "modifier", map.modifier_value(id, modifier.id()))?,
                "has" => {
                    writeln!(out, "has {}\nok", map.has(id))?;
                }
                "has_modifier" => {
                    writeln!(
                        out,
                        "has_modifier {}\nok",
                        map.has_modifier(id, modifier.id())
                    )?;
                }
                "ensure" => {
                    writeln!(out, "ensure {}\nok", map.ensure(id))?;
                }
                "reset" => {
                    writeln!(out, "reset {}\nok", map.reset_base(id))?;
                }
                "ack_update" => {
                    map.clear_update(id);
                    writeln!(out, "ok")?;
                }
                "ack_sync" => {
                    map.clear_sync(id);
                    writeln!(out, "ok")?;
                }
                _ => {
                    let mut removed = None;
                    let result = map.edit(id, |a, o| match mode {
                        "set" => a.set_base_value(number(&op["base"]), o),
                        "add" => a.add_transient(modifier.clone(), o),
                        "update" => a.add_or_update_transient(modifier.clone(), o),
                        "permanent" => a.add_permanent(modifier.clone(), o),
                        "remove" => a.remove(modifier.id(), o).map(|v| {
                            removed = Some(v);
                        }),
                        "clear" => a.remove_all(o),
                        _ => panic!("mode"),
                    });
                    if let Some(v) = removed {
                        writeln!(out, "removed {v}")?;
                    }
                    match result {
                        None => writeln!(out, "absent\nok")?,
                        Some(Ok(())) => writeln!(out, "ok")?,
                        Some(Err(AttributeError::DuplicateModifier(_))) => {
                            writeln!(out, "error duplicate")?
                        }
                        Some(Err(AttributeError::Observer(e))) => match e {},
                    }
                }
            }
            for (i, id) in ids.iter().copied().enumerate() {
                if let Some(a) = map.instance(id) {
                    writeln!(
                        out,
                        "state {i} {:016x} {} {:016x} {} {} {} {}",
                        f64_bits(a.base_value()),
                        a.is_dirty(),
                        f64_bits(a.cached_value()),
                        map.needs_update(id),
                        map.needs_sync(id),
                        a.modifiers().len(),
                        a.permanent_modifiers().len()
                    )?;
                }
            }
            writeln!(out, "end")?;
        }
    }
    Ok(())
}
