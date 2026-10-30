use horizon_world::object_open_set::{CursorError, ObjectKey, ObjectOpenSet};
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
        let mut set = ObjectOpenSet::default();
        let mut cursor = set.cursor();
        for op in f["ops"].as_array().unwrap() {
            let index = op[1].as_i64().unwrap();
            let key = (index >= 0).then(|| &keys[index as usize]);
            let result: Result<String, CursorError> = match op[0].as_str().unwrap() {
                "add" => Ok(format!("add {}", set.add(key.cloned()))),
                "remove" => Ok(format!("remove {}", set.remove(key))),
                "contains" => Ok(format!("contains {}", set.contains(key))),
                "get" => Ok(format!("get {}", desc(key.and_then(|k| set.get(k))))),
                "clear" => {
                    set.clear();
                    Ok("clear".into())
                }
                "cursor" => {
                    cursor = set.cursor();
                    Ok("cursor".into())
                }
                "next" | "force-next" => {
                    if cursor.has_next() || op[0] == "force-next" {
                        set.next(&mut cursor)
                            .map(|k| format!("next {}", desc(k.as_ref())))
                    } else {
                        Ok("next none".into())
                    }
                }
                "delete" => set.remove_current(&mut cursor).map(|()| "delete".into()),
                _ => panic!("operation"),
            };
            writeln!(
                out,
                "{}",
                match result {
                    Ok(v) => v,
                    Err(CursorError::NoSuchElement) => "next none".into(),
                    Err(CursorError::NoCurrentEntry) => "error current".into(),
                    Err(CursorError::MissingWrapped) => "error null".into(),
                    Err(CursorError::IndexOutOfBounds) => "error bounds".into(),
                }
            )?;
            writeln!(
                out,
                "set {} {} {} | {}",
                set.len(),
                set.is_empty(),
                cursor.has_next(),
                set.values().map(desc).collect::<Vec<_>>().join(",")
            )?;
        }
    }
    Ok(())
}
