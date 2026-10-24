use crate::support::common::f64_bits;
use horizon_world::damage_source::{Catalog, EntityKind, SourceWorld};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
fn number(v: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
fn position(v: &Value) -> [f64; 3] {
    std::array::from_fn(|i| number(&v[i]))
}
fn vec(v: Option<[f64; 3]>) -> String {
    match v {
        None => "none".into(),
        Some(v) => format!(
            "{:016x} {:016x} {:016x}",
            f64_bits(v[0]),
            f64_bits(v[1]),
            f64_bits(v[2])
        ),
    }
}
struct World<'a> {
    f: &'a Value,
    out: &'a mut dyn Write,
    calls: i64,
    changed: bool,
}
impl World<'_> {
    fn state(&self) -> &Value {
        &self.f[if self.changed { "after" } else { "initial" }]
    }
    fn event(&mut self, s: &str) -> io::Result<()> {
        writeln!(self.out, "{s}")?;
        let n = self.calls;
        self.calls += 1;
        if n == self.f["fail"].as_i64().unwrap() {
            return Err(io::Error::other("injected damage source query failure"));
        }
        if n == self.f["mutate_at"].as_i64().unwrap() {
            self.changed = true;
        }
        Ok(())
    }
}
impl SourceWorld for World<'_> {
    type Error = io::Error;
    type Entity = usize;
    type Weapon = ();
    fn kind(&self, id: usize) -> EntityKind {
        match id {
            2 => EntityKind::Living,
            3 | 4 => EntityKind::Player,
            _ => EntityKind::Other,
        }
    }
    fn weapon(&mut self, id: usize) -> io::Result<Option<()>> {
        self.event(&format!("weapon {id}"))?;
        Ok(self.state()["weapons"][id].as_bool().unwrap().then_some(()))
    }
    fn position(&mut self, id: usize) -> io::Result<[f64; 3]> {
        self.event(&format!("position {id}"))?;
        Ok(position(&self.state()["positions"][id]))
    }
    fn causing_entity(&mut self, stored: Option<usize>) -> io::Result<Option<usize>> {
        self.event("entity")?;
        Ok(if self.f["virtual"].as_bool().unwrap() {
            let id = self.state()["virtual_entity"].as_u64().unwrap() as usize;
            (id != 0).then_some(id)
        } else {
            stored
        })
    }
    fn instabuild(&mut self, id: usize) -> io::Result<bool> {
        self.event(&format!("abilities {id}"))?;
        Ok(self.state()["creative"][id].as_bool().unwrap())
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).unwrap();
    let dir = Path::new(&arg);
    let catalog = Catalog::from_json(
        &serde_json::from_str(&fs::read_to_string(dir.join("catalog.json"))?)?,
        &serde_json::from_str(&fs::read_to_string(dir.join("tags.json"))?)?,
    )?;
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for (id, d) in catalog.entries() {
        writeln!(
            out,
            "definition {} {} {:08x} {} {} {}",
            d.name,
            d.message_id,
            d.exhaustion.to_bits(),
            d.scaling.name(),
            d.effects,
            d.death_message_type
        )?;
        for tag in catalog.tags() {
            writeln!(out, "tag {tag} {}", catalog.in_tag(id, tag))?;
        }
    }
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let entity = |key: &str| {
            let id = f[key].as_u64().unwrap() as usize;
            (id != 0).then_some(id)
        };
        let (direct, causing, pos) = match f["constructor"].as_u64().unwrap() {
            0 => (entity("direct"), entity("causing"), None),
            1 => (entity("direct"), entity("direct"), None),
            2 => (None, None, Some(position(&f["position"]))),
            3 => (None, None, None),
            _ => panic!("constructor"),
        };
        let source = catalog
            .source(
                catalog.id(f["type"].as_str().unwrap()).unwrap(),
                direct,
                causing,
                pos,
            )
            .unwrap();
        let mut w = World {
            f: &f,
            out: &mut out,
            calls: 0,
            changed: false,
        };
        let result = (|| -> io::Result<()> {
            writeln!(
                w.out,
                "direct {} {} {}",
                source.is_direct(),
                source.direct_entity().unwrap_or(0),
                source.causing_entity().unwrap_or(0)
            )?;
            let scales = source.scales_with_difficulty(&w);
            writeln!(w.out, "scales {scales}")?;
            let weapon = source.weapon(&mut w)?;
            writeln!(w.out, "weapon-result {}", weapon.is_some())?;
            let creative = source.creative_player(&mut w)?;
            writeln!(w.out, "creative {creative}")?;
            let pos = source.position(&mut w)?;
            writeln!(w.out, "position-result {}", vec(pos))?;
            writeln!(w.out, "raw {}", vec(source.position_raw()))?;
            writeln!(
                w.out,
                "type {}",
                source.is_type(f["check_type"].as_str().unwrap())
            )?;
            Ok(())
        })();
        writeln!(
            w.out,
            "result {}",
            if result.is_ok() { "ok" } else { "error" }
        )?;
        writeln!(w.out, "queries {} {}", w.calls, w.changed)?;
    }
    Ok(())
}
