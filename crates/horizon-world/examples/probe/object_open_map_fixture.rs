use horizon_world::{
    object_open_map::{Entry, ObjectOpenMap, TraversalError},
    object_open_set::{CursorError, ObjectKey},
};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};

#[derive(Clone)]
struct Key {
    id: usize,
    hash: i32,
    object: usize,
}
impl PartialEq for Key {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}
impl Eq for Key {}
impl ObjectKey for Key {
    fn object_hash(&self) -> i32 {
        self.hash
    }
}
fn desc(key: Option<&Key>) -> String {
    key.map_or("null".into(), |k| format!("{}:{}", k.id, k.object))
}
fn value(v: Option<i32>) -> String {
    v.map_or("null".into(), |v| v.to_string())
}
fn error(e: CursorError) -> String {
    match e {
        CursorError::NoSuchElement => "next none",
        CursorError::NoCurrentEntry => "error current",
        CursorError::MissingWrapped => "error null",
        CursorError::IndexOutOfBounds => "error bounds",
    }
    .into()
}
fn snapshot(map: &ObjectOpenMap<Key, i32>) -> String {
    format!(
        "map {} {} | {}",
        map.len(),
        map.is_empty(),
        map.entries()
            .iter()
            .map(|(k, v)| format!("{}={}", desc(k.as_ref()), value(*v)))
            .collect::<Vec<_>>()
            .join(",")
    )
}
fn key<'a>(keys: &'a [Key], op: &Value) -> Option<&'a Key> {
    let index = op[1].as_i64().unwrap();
    (index >= 0).then(|| &keys[index as usize])
}
fn mutation(map: &mut ObjectOpenMap<Key, i32>, keys: &[Key], op: &Value) -> String {
    let k = key(keys, op);
    match op[0].as_str().unwrap() {
        "put" => format!(
            "put {}",
            value(map.put(k.cloned(), op[2].as_i64().map(|v| v as i32)))
        ),
        "remove" => format!("remove {}", value(map.remove(k))),
        "contains" => format!("contains {}", map.contains_key(k)),
        "get" => format!("get {}", value(map.get(k))),
        "clear" => {
            map.clear();
            "clear".into()
        }
        _ => panic!("mutation"),
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).unwrap();
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let keys: Vec<_> = f["hashes"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .map(|(object, h)| Key {
                id: f["ids"][object].as_u64().unwrap() as usize,
                hash: h.as_i64().unwrap() as i32,
                object,
            })
            .collect();
        let mut map = ObjectOpenMap::default();
        let mut cursor = map.cursor();
        let mut entries: Vec<Entry> = Vec::new();
        for op in f["ops"].as_array().unwrap() {
            let result: Result<String, CursorError> = match op[0].as_str().unwrap() {
                "put" | "remove" | "contains" | "get" | "clear" => {
                    Ok(mutation(&mut map, &keys, op))
                }
                "cursor" => {
                    cursor = map.cursor();
                    Ok("cursor".into())
                }
                "next" | "force-next" => {
                    if cursor.has_next() || op[0] == "force-next" {
                        map.next_entry(&mut cursor).and_then(|entry| {
                            entries.push(entry.clone());
                            Ok(format!(
                                "next {} {}={}",
                                entries.len() - 1,
                                desc(map.entry_key(&entry)?.as_ref()),
                                value(map.entry_value(&entry)?)
                            ))
                        })
                    } else {
                        Ok("next none".into())
                    }
                }
                "delete" => map.remove_current(&mut cursor).map(|()| "delete".into()),
                "entry-key" | "entry-value" | "entry-set" => entries
                    .get(op[1].as_u64().unwrap() as usize)
                    .ok_or(CursorError::IndexOutOfBounds)
                    .and_then(|e| match op[0].as_str().unwrap() {
                        "entry-key" => map
                            .entry_key(e)
                            .map(|k| format!("entry-key {}", desc(k.as_ref()))),
                        "entry-value" => map
                            .entry_value(e)
                            .map(|v| format!("entry-value {}", value(v))),
                        _ => map
                            .set_entry_value(e, op[2].as_i64().map(|v| v as i32))
                            .map(|v| format!("entry-set {}", value(v))),
                    }),
                "each" => {
                    let mut call = 0usize;
                    let mut events = Vec::new();
                    let result = map.for_each(|map, k, v| {
                        events.push(format!("visit {call} {}={}", desc(k.as_ref()), value(v)));
                        if let Some(actions) = op[1][call].as_array() {
                            for action in actions {
                                if action[0] == "fail" {
                                    return Err(());
                                }
                                events.push(format!("callback {}", mutation(map, &keys, action)));
                                events.push(snapshot(map));
                            }
                        }
                        call += 1;
                        Ok(())
                    });
                    for event in events {
                        writeln!(out, "{event}")?;
                    }
                    match result {
                        Ok(()) => Ok("each".into()),
                        Err(TraversalError::Callback(())) => Ok("error callback".into()),
                        Err(TraversalError::Cursor(e)) => Err(e),
                    }
                }
                _ => panic!("operation"),
            };
            writeln!(out, "{}", result.unwrap_or_else(error))?;
            writeln!(out, "{} | cursor {}", snapshot(&map), cursor.has_next())?;
        }
    }
    Ok(())
}
