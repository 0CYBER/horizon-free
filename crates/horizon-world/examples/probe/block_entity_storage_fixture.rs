use horizon_core::position::{BlockPos, ChunkPos};
use horizon_world::{
    block_entities::{ProtoBlockEntities, position_from_tag},
    nbt::Tag,
};
use std::{
    fs,
    io::{self, BufRead, Write},
    str::SplitWhitespace,
};
struct Input<'a>(SplitWhitespace<'a>);
impl Input<'_> {
    fn int(&mut self) -> i32 {
        self.0.next().unwrap().parse().unwrap()
    }
    fn long(&mut self) -> i64 {
        self.0.next().unwrap().parse().unwrap()
    }
    fn pos(&mut self) -> BlockPos {
        BlockPos::new(self.int(), self.int(), self.int())
    }
    fn tag(&mut self) -> Option<Tag> {
        let kind = self.0.next().unwrap();
        let text = self.0.next().unwrap();
        Some(match kind {
            "none" => return None,
            "byte" => Tag::Byte(text.parse().unwrap()),
            "short" => Tag::Short(text.parse().unwrap()),
            "int" => Tag::Int(text.parse().unwrap()),
            "long" => Tag::Long(text.parse().unwrap()),
            "float" => {
                let bits = u32::from_str_radix(text, 16).unwrap();
                Tag::Float(if f32::from_bits(bits) == 0.0 { 0 } else { bits })
            }
            "double" => {
                let bits = u64::from_str_radix(text, 16).unwrap();
                Tag::Double(if f64::from_bits(bits) == 0.0 { 0 } else { bits })
            }
            "string" => Tag::String(text.into()),
            _ => panic!("unknown atom"),
        })
    }
}
fn xyz(p: BlockPos) -> String {
    format!("{} {} {}", p.x, p.y, p.z)
}
fn atom(tag: Option<&Tag>) -> String {
    match tag {
        None => "none".into(),
        Some(Tag::Float(v)) => format!("float:{v:x}"),
        Some(Tag::Double(v)) => format!("double:{v:x}"),
        Some(Tag::Byte(v)) => format!("1:{v}"),
        Some(Tag::Short(v)) => format!("2:{v}"),
        Some(Tag::Int(v)) => format!("3:{v}"),
        Some(Tag::Long(v)) => format!("4:{v}"),
        Some(v) => format!("{}:invalid", v.id()),
    }
}
fn pending(tag: Option<&Tag>) -> String {
    match tag {
        None => "none".into(),
        Some(tag) => {
            let Some(Tag::Long(mark)) = tag.get("mark") else {
                panic!("missing marker")
            };
            format!(
                "{mark} {} {} {}",
                atom(tag.get("x")),
                atom(tag.get("y")),
                atom(tag.get("z"))
            )
        }
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = crate::args().collect();
    let mut out = io::BufWriter::new(fs::File::create(&args[2])?);
    let mut position = ChunkPos::default();
    let mut chunk = ProtoBlockEntities::<i64>::new(position);
    for (index, line) in io::BufReader::new(fs::File::open(&args[1])?)
        .lines()
        .enumerate()
    {
        let line = line?;
        let mut input = Input(line.split_whitespace());
        let op = input.0.next().unwrap();
        writeln!(out, "command {index} {op}")?;
        match op {
            "reset" => {
                position = ChunkPos::new(input.int(), input.int());
                chunk = ProtoBlockEntities::new(position);
            }
            "pending" | "resolve" => {
                let mut entries = std::collections::BTreeMap::new();
                entries.insert("mark".into(), Tag::Long(input.long()));
                for name in ["x", "y", "z"] {
                    if let Some(tag) = input.tag() {
                        entries.insert(name.into(), tag);
                    }
                }
                let tag = Tag::Compound(entries);
                writeln!(out, "position {}", xyz(position_from_tag(position, &tag)?))?;
                if op == "pending" {
                    chunk.set_pending(tag)?;
                }
            }
            "active" => {
                let p = input.pos();
                chunk.set_active(p, input.long());
            }
            "remove" => {
                chunk.remove(input.pos());
            }
            "mutate" => {
                let p = input.pos();
                let x = input.int();
                if let Some(Tag::Compound(map)) = chunk.pending_mut(p) {
                    map.insert("x".into(), Tag::Int(x));
                }
            }
            "get" => {
                let p = input.pos();
                writeln!(
                    out,
                    "active {}",
                    chunk.active(p).map_or("none".into(), ToString::to_string)
                )?;
                writeln!(out, "pending {}", pending(chunk.pending(p)))?;
            }
            "save" => {
                let p = input.pos();
                let saved = chunk.for_saving(p, |mark| {
                    Ok::<_, io::Error>(Tag::compound([("LootTableSeed", Tag::Long(*mark))]))
                })?;
                if chunk.active(p).is_some() {
                    let Some(Tag::Long(seed)) = saved.as_ref().unwrap().get("LootTableSeed") else {
                        unreachable!()
                    };
                    writeln!(out, "saved active minecraft:chest {} {seed}", xyz(p))?;
                } else {
                    writeln!(out, "saved pending {}", pending(saved.as_ref()))?;
                }
            }
            "dump" => {
                let mut positions: Vec<_> = chunk.positions().into_iter().collect();
                positions.sort_by_key(|p| (p.x, p.y, p.z));
                for p in &positions {
                    writeln!(
                        out,
                        "entry {} {} {}",
                        xyz(*p),
                        chunk.active(*p).map_or("none".into(), ToString::to_string),
                        pending(chunk.pending(*p))
                    )?;
                }
                writeln!(
                    out,
                    "size {} {}",
                    positions.len(),
                    chunk.pending_entries().len()
                )?;
            }
            _ => panic!("unknown command"),
        }
        assert!(input.0.next().is_none());
    }
    out.flush()?;
    Ok(())
}
