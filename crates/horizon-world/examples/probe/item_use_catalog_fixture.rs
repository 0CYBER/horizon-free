use horizon_world::item_use_catalog::{ItemUseCatalog, ItemUseStack};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
fn float(v: &Value) -> Option<f32> {
    v.as_str()
        .map(|s| f32::from_bits(u32::from_str_radix(s, 16).unwrap()))
}
fn snapshot(out: &mut dyn Write, s: &ItemUseStack<'_>) -> io::Result<()> {
    let d = s.durability();
    writeln!(
        out,
        "stack {} {} {} {} {} {} {} {} {} {} {} {} {}",
        if s.is_empty() {
            0
        } else {
            s.definition().id().raw()
        },
        d.count(),
        s.max_stack_size(),
        d.max_damage(),
        d.damage(),
        s.is_empty(),
        d.is_damageable(),
        d.is_damaged(),
        d.is_broken(),
        d.next_damage_will_break(),
        s.is_stackable(),
        s.use_duration(),
        s.use_on_release()
    )
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).unwrap();
    let dir = Path::new(&arg);
    let catalog = ItemUseCatalog::from_json(&serde_json::from_str(&fs::read_to_string(
        dir.join("catalog.json"),
    )?)?)?;
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let mut s = catalog
            .by_name(f["name"].as_str().unwrap())
            .unwrap()
            .stack(f["count"].as_i64().unwrap() as i32);
        snapshot(&mut out, &s)?;
        for op in f["ops"].as_array().unwrap() {
            let v = &op["value"];
            match op["kind"].as_str().unwrap() {
                "count" => s.count = v.as_i64().unwrap() as i32,
                "set-damage" => {
                    let mut d = s.durability();
                    d.set_damage(v.as_i64().unwrap() as i32);
                    s.components.damage = d.damage;
                }
                "damage" => s.components.damage = v.as_i64().map(|v| v as i32),
                "max-damage" => s.components.max_damage = v.as_i64().map(|v| v as i32),
                "max-stack" => s.components.max_stack_size = v.as_i64().map(|v| v as i32),
                "unbreakable" => s.components.unbreakable = v.as_bool().unwrap(),
                "consume" => s.components.consume_seconds = float(v),
                "blocking" => s.components.block_delay_seconds = float(v),
                "kinetic" => s.components.kinetic_weapon = v.as_bool().unwrap(),
                "instrument" => s.components.instrument_seconds = float(v),
                _ => panic!("operation"),
            };
            snapshot(&mut out, &s)?;
        }
    }
    Ok(())
}
