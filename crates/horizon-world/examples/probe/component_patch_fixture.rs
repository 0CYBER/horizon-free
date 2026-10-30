use horizon_world::component_patch::{ComponentMap, Patch, Prototype};
use serde_json::Value as Json;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
#[derive(Clone, Debug, PartialEq)]
enum Value {
    Int(i32),
    Float(u32),
}
fn value(v: &Json) -> Option<Value> {
    if v.is_null() {
        None
    } else if let Some(s) = v.as_str() {
        let raw = u32::from_str_radix(s, 16).unwrap();
        Some(Value::Float(if f32::from_bits(raw).is_nan() {
            f32::NAN.to_bits()
        } else {
            raw
        }))
    } else {
        Some(Value::Int(v.as_i64().unwrap() as i32))
    }
}
fn text(v: Option<&Value>) -> String {
    match v {
        None => "none".into(),
        Some(Value::Int(v)) => format!("i{v}"),
        Some(Value::Float(v)) => format!("f{v:08x}"),
    }
}
fn patch(v: &Json) -> Patch<usize, Value> {
    Patch::new(
        v.as_array()
            .unwrap()
            .iter()
            .map(|v| (v[0].as_u64().unwrap() as usize, value(&v[1]))),
    )
}
fn patch_text(p: &Patch<usize, Value>) -> String {
    p.entries()
        .iter()
        .map(|(k, v)| format!("{k}={}", text(v.as_ref())))
        .collect::<Vec<_>>()
        .join(",")
}
fn snapshot(
    out: &mut dyn Write,
    maps: &[ComponentMap<usize, Value>; 3],
    patches: &[Patch<usize, Value>; 3],
) -> io::Result<()> {
    for (i, m) in maps.iter().enumerate() {
        let entries = m
            .entries()
            .map(|(k, v)| format!("{k}={}", text(Some(v))))
            .collect::<Vec<_>>()
            .join(",");
        let keys = m
            .keys()
            .iter()
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let values = (0..16)
            .map(|k| text(m.get(&k)))
            .collect::<Vec<_>>()
            .join(",");
        let flags = (0..16)
            .filter(|k| m.has_non_default(k))
            .fold(0u32, |v, k| v | (1 << k));
        writeln!(
            out,
            "map {i} {} {entries} | {keys} | {values} | {flags} | {}",
            m.len(),
            patch_text(&m.as_patch())
        )?;
    }
    for (i, p) in patches.iter().enumerate() {
        writeln!(out, "patch {i} {}", patch_text(p))?;
    }
    writeln!(
        out,
        "equal {} {} {}",
        maps[0] == maps[1],
        maps[0] == maps[2],
        maps[1] == maps[2]
    )
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).unwrap();
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Json = serde_json::from_str(line)?;
        let base = Prototype::new(
            f["prototype"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| (v[0].as_u64().unwrap() as usize, value(&v[1]).unwrap())),
        );
        let mut maps = std::array::from_fn(|_| ComponentMap::new(base.clone()));
        let mut patches = std::array::from_fn(|_| Patch::new([]));
        snapshot(&mut out, &maps, &patches)?;
        for op in f["ops"].as_array().unwrap() {
            let a = op["a"].as_u64().unwrap() as usize;
            let b = op["b"].as_u64().unwrap() as usize;
            let mut success = true;
            match op["kind"].as_str().unwrap() {
                "set" => {
                    match maps[a].set(op["key"].as_u64().unwrap() as usize, value(&op["value"])) {
                        Ok(v) => writeln!(out, "old {}", text(v.as_ref()))?,
                        Err(_) => success = false,
                    }
                }
                "remove" => {
                    let v = maps[a].remove(op["key"].as_u64().unwrap() as usize);
                    writeln!(out, "old {}", text(v.as_ref()))?;
                }
                "copy" => maps[b] = maps[a].clone(),
                "save" => patches[b] = maps[a].as_patch(),
                "apply" => maps[a].apply_patch(&patches[b]),
                "restore" => maps[a].restore_patch(&patches[b]),
                "clear" => maps[a].clear_patch(),
                "build" => patches[b] = patch(&op["entries"]),
                "from" => maps[a] = ComponentMap::from_patch(base.clone(), &patches[b]),
                _ => panic!("operation"),
            };
            writeln!(out, "result {}", if success { "ok" } else { "error" })?;
            snapshot(&mut out, &maps, &patches)?;
        }
    }
    Ok(())
}
