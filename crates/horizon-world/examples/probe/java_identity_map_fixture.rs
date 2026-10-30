use horizon_world::java_hash_map::{CursorError, JavaHashMap, JavaKey};
use serde_json::Value;
use std::{
    cmp::Ordering,
    fs,
    io::{self, Write},
    path::Path,
};
#[derive(Clone)]
struct Key {
    object: usize,
    id: i32,
    hash: i32,
    kind: i32,
    identity: i32,
}
impl PartialEq for Key {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind && self.id == other.id
    }
}
impl Eq for Key {}
impl JavaKey for Key {
    fn java_hash(&self) -> i32 {
        self.hash
    }
    fn comparable_compare(&self, other: &Self) -> Option<Ordering> {
        if self.kind != other.kind {
            None
        } else {
            match self.kind {
                2 => Some((self.id % 4).cmp(&(other.id % 4))),
                3 => Some(self.id.cmp(&other.id)),
                _ => None,
            }
        }
    }
    fn tie_break(&self, other: &Self) -> Ordering {
        self.kind.cmp(&other.kind).then_with(|| {
            if self.identity <= other.identity {
                Ordering::Less
            } else {
                Ordering::Greater
            }
        })
    }
}
fn opt(v: Option<i32>) -> String {
    v.map_or("none".into(), |v| v.to_string())
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).unwrap();
    let dir = Path::new(&arg);
    let side = crate::args().nth(2).unwrap();
    let mut out = io::BufWriter::new(fs::File::create(dir.join(format!("rust-{side}.txt")))?);
    for line in fs::read_to_string(dir.join(format!("inputs-{side}.txt")))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let keys: Vec<_> = f["keys"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .map(|(object, v)| Key {
                object,
                id: v["id"].as_i64().unwrap() as i32,
                hash: v["hash"].as_i64().unwrap() as i32,
                kind: v["kind"].as_i64().unwrap() as i32,
                identity: f["identities"][object].as_i64().unwrap() as i32,
            })
            .collect();
        let mut map = JavaHashMap::default();
        let mut cursor = map.cursor();
        let mut saved = JavaHashMap::default();
        for op in f["ops"].as_array().unwrap() {
            let k = op[1].as_u64().unwrap() as usize;
            let result: Result<String, CursorError> = match op[0].as_str().unwrap() {
                "put" => Ok(format!(
                    "old {}",
                    opt(map.put(keys[k].clone(), op[2].as_i64().unwrap() as i32))
                )),
                "remove" => Ok(format!("old {}", opt(map.remove(&keys[k])))),
                "get" => Ok(format!("get {}", opt(map.get(&keys[k]).copied()))),
                "copy" => {
                    map = map.copied();
                    cursor = map.cursor();
                    Ok("copy".into())
                }
                "save" => {
                    saved = map.copied();
                    Ok("save".into())
                }
                "swap" => {
                    std::mem::swap(&mut map, &mut saved);
                    cursor = map.cursor();
                    Ok("swap".into())
                }
                "clear" => {
                    map.clear();
                    Ok("clear".into())
                }
                "cursor" => {
                    cursor = map.cursor();
                    Ok("cursor".into())
                }
                "next" | "force-next" => {
                    if cursor.has_next() || op[0] == "force-next" {
                        map.next_entry(&mut cursor).map(|e| {
                            e.map_or("next none".into(), |(k, v)| {
                                format!("next {}={v}", k.object)
                            })
                        })
                    } else {
                        Ok("next none".into())
                    }
                }
                "delete" => map.remove_current(&mut cursor).map(|()| "delete".into()),
                _ => panic!(),
            };
            writeln!(
                out,
                "{}",
                match result {
                    Ok(s) => s,
                    Err(CursorError::ConcurrentModification) => "error modified".into(),
                    Err(CursorError::NoCurrentEntry) => "error current".into(),
                    Err(e) => panic!("{e:?}"),
                }
            )?;
            for (name, current) in [("map", &map), ("saved", &saved)] {
                let entries = current
                    .entries()
                    .map(|(k, v)| format!("{}={v}", k.object))
                    .collect::<Vec<_>>()
                    .join(",");
                writeln!(
                    out,
                    "{name} {} {} {} | {}",
                    current.len(),
                    current.is_empty(),
                    cursor.has_next(),
                    entries
                )?;
            }
        }
    }
    Ok(())
}
