use horizon_world::java_hash_map::{CursorError, JavaHashMap, JavaKey};
use serde_json::Value;
use std::{
    cmp::Ordering,
    fs,
    io::{self, Write},
    path::Path,
};
#[derive(Clone, Eq, PartialEq)]
struct Key {
    id: usize,
    hash: i32,
}
impl Ord for Key {
    fn cmp(&self, other: &Self) -> Ordering {
        self.id.cmp(&other.id)
    }
}
impl PartialOrd for Key {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl JavaKey for Key {
    fn java_hash(&self) -> i32 {
        self.hash
    }
    fn comparable_compare(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
    fn tie_break(&self, other: &Self) -> Ordering {
        self.cmp(other)
    }
}
fn opt(v: Option<i32>) -> String {
    v.map_or("none".into(), |v| v.to_string())
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
            .map(|(id, v)| Key {
                id,
                hash: v.as_i64().unwrap() as i32,
            })
            .collect();
        let mut map = JavaHashMap::default();
        let mut cursor = map.cursor();
        for op in f["ops"].as_array().unwrap() {
            let k = op[1].as_u64().unwrap() as usize;
            let result: Result<String, CursorError> = match op[0].as_str().unwrap() {
                "put" => Ok(format!(
                    "old {}",
                    opt(map.put(keys[k].clone(), op[2].as_i64().unwrap() as i32))
                )),
                "remove" => Ok(format!("old {}", opt(map.remove(&keys[k])))),
                "get" => Ok(format!("get {}", opt(map.get(&keys[k]).copied()))),
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
                            e.map_or("next none".into(), |(k, v)| format!("next {}={v}", k.id))
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
            let entries = map
                .entries()
                .map(|(k, v)| format!("{}={v}", k.id))
                .collect::<Vec<_>>()
                .join(",");
            writeln!(
                out,
                "map {} {} {} | {}",
                map.len(),
                map.is_empty(),
                cursor.has_next(),
                entries
            )?;
        }
    }
    Ok(())
}
