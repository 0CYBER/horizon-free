use crate::support::common::f32_bits;
use horizon_world::{
    attributes::{
        Modifier, Operation,
        entity::{Catalog, EntityAttributes},
    },
    player_health::{PlayerAttributeHealth, PlayerFloatIds},
    synced_floats::{FloatValue, Observer, SyncedFloats},
};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
    sync::Arc,
};
fn single(v: &Value) -> f32 {
    f32::from_bits(u32::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
fn number(v: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
fn values(v: Option<&[FloatValue]>) -> String {
    match v {
        None => "none".into(),
        Some([]) => "empty".into(),
        Some(v) => v
            .iter()
            .map(|v| format!("{}:{:08x}", v.id, f32_bits(v.value)))
            .collect::<Vec<_>>()
            .join(","),
    }
}
struct Callback<'a> {
    out: &'a mut dyn Write,
    op: &'a Value,
    calls: i64,
}
impl Callback<'_> {
    fn event(&mut self, s: &str) -> io::Result<()> {
        writeln!(self.out, "{s}")?;
        let n = self.calls;
        self.calls += 1;
        if n == self.op["fail"].as_i64().unwrap() {
            Err(io::Error::other("injected player health callback failure"))
        } else {
            Ok(())
        }
    }
}
impl Observer for Callback<'_> {
    type Error = io::Error;
    fn updated(&mut self, data: &mut SyncedFloats, id: u8) -> io::Result<()> {
        self.event(&format!(
            "updated {id} {:08x} {}",
            f32_bits(data.get(id).unwrap()),
            data.is_dirty()
        ))?;
        if self.op["pack_callback"].as_bool().unwrap() {
            writeln!(
                self.out,
                "callback-pack {}",
                values(data.take_dirty().as_deref())
            )?;
        }
        Ok(())
    }
    fn assigned(&mut self, _: &mut SyncedFloats, v: &[FloatValue]) -> io::Result<()> {
        self.event(&format!("assigned {}", values(Some(v))))
    }
}
struct State {
    attributes: EntityAttributes,
    data: SyncedFloats,
    ids: PlayerFloatIds,
    dead: bool,
}
impl State {
    fn bind<'a, 'b>(
        &'a mut self,
        cb: &'a mut Callback<'b>,
    ) -> PlayerAttributeHealth<'a, Callback<'b>> {
        PlayerAttributeHealth::new(
            &mut self.attributes,
            &mut self.data,
            self.ids,
            self.dead,
            cb,
        )
        .unwrap()
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).unwrap();
    let dir = Path::new(&arg);
    let ids: Value = serde_json::from_str(&fs::read_to_string(dir.join("ids.json"))?)?;
    let ids = PlayerFloatIds {
        health: ids["health"].as_u64().unwrap() as u8,
        absorption: ids["absorption"].as_u64().unwrap() as u8,
    };
    let catalog = Arc::new(Catalog::from_json(&serde_json::from_str(
        &fs::read_to_string(dir.join("catalog.json"))?,
    )?)?);
    let keys = [
        catalog.id("minecraft:max_health").unwrap(),
        catalog.id("minecraft:max_absorption").unwrap(),
    ];
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let mut state = State {
            attributes: catalog.create("minecraft:player").unwrap(),
            data: SyncedFloats::new(&[
                FloatValue {
                    id: ids.health,
                    value: single(&f["health"]),
                },
                FloatValue {
                    id: ids.absorption,
                    value: single(&f["absorption"]),
                },
            ])?,
            ids,
            dead: f["dead"].as_bool().unwrap(),
        };
        for op in f["ops"].as_array().unwrap() {
            let mut cb = Callback {
                out: &mut out,
                op,
                calls: 0,
            };
            let key = keys[op["attribute"].as_u64().unwrap() as usize];
            let v = single(&op["value"]);
            let result = match op["mode"].as_str().unwrap() {
                "set" => state.bind(&mut cb).set_health(v).map_err(|_| ()),
                "heal" => state.bind(&mut cb).heal(v).map_err(|_| ()),
                "absorption" => state.bind(&mut cb).set_absorption(v).map_err(|_| ()),
                "get-absorption" => state
                    .bind(&mut cb)
                    .absorption()
                    .map_err(|_| ())
                    .and_then(|v| writeln!(cb.out, "float {:08x}", f32_bits(v)).map_err(|_| ())),
                "hurt" => state
                    .bind(&mut cb)
                    .is_hurt()
                    .map_err(|_| ())
                    .and_then(|v| writeln!(cb.out, "bool {v}").map_err(|_| ())),
                "dead" => state
                    .bind(&mut cb)
                    .is_dead_or_dying()
                    .map_err(|_| ())
                    .and_then(|v| writeln!(cb.out, "bool {v}").map_err(|_| ())),
                "base" => {
                    state
                        .attributes
                        .edit(key, |a, o| a.set_base_value(number(&op["amount"]), o))
                        .unwrap()
                        .unwrap();
                    Ok(())
                }
                "add" => {
                    let m = Modifier::new(
                        op["id"].as_str().unwrap(),
                        number(&op["amount"]),
                        Operation::ALL[op["operation"].as_u64().unwrap() as usize],
                    )?;
                    state
                        .attributes
                        .edit(key, |a, o| a.add_or_update_transient(m, o))
                        .unwrap()
                        .unwrap();
                    Ok(())
                }
                "remove" => {
                    state
                        .attributes
                        .edit(key, |a, o| a.remove(op["id"].as_str().unwrap(), o))
                        .unwrap()
                        .unwrap();
                    Ok(())
                }
                "reset" => {
                    state.attributes.reset_base(key);
                    Ok(())
                }
                "pack" => writeln!(
                    cb.out,
                    "packed {}",
                    values(state.data.take_dirty().as_deref())
                )
                .map_err(|_| ()),
                "defaults" => writeln!(
                    cb.out,
                    "defaults {}",
                    values(state.data.non_default().as_deref())
                )
                .map_err(|_| ()),
                "assign" => {
                    let values = op["values"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|v| FloatValue {
                            id: if v["field"] == 0 {
                                ids.health
                            } else {
                                ids.absorption
                            },
                            value: single(&v["value"]),
                        })
                        .collect::<Vec<_>>();
                    state.data.assign(&values, &mut cb).map_err(|_| ())
                }
                _ => panic!("mode"),
            };
            writeln!(
                cb.out,
                "result {}",
                if result.is_ok() { "ok" } else { "error" }
            )?;
            writeln!(
                cb.out,
                "state {:08x} {:08x} {} {} {} {} {} {}",
                f32_bits(state.data.get(ids.health).unwrap()),
                f32_bits(state.data.get(ids.absorption).unwrap()),
                state.data.is_dirty(),
                state.data.item_dirty(ids.health).unwrap(),
                state.data.item_dirty(ids.absorption).unwrap(),
                state.data.is_default(ids.health).unwrap(),
                state.data.is_default(ids.absorption).unwrap(),
                cb.calls
            )?;
        }
    }
    Ok(())
}
