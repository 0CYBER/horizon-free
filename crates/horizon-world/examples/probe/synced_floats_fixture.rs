use crate::support::common::f32_bits;
use horizon_world::synced_floats::{DataError, FloatValue, Observer, SyncedFloats};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
fn single(v: &Value) -> f32 {
    f32::from_bits(u32::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
fn parse(v: &Value) -> Vec<FloatValue> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|v| FloatValue {
            id: v["id"].as_u64().unwrap() as u8,
            value: single(&v["value"]),
        })
        .collect()
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
    nested: bool,
}
impl Callback<'_> {
    fn event(&mut self, s: &str) -> io::Result<()> {
        writeln!(self.out, "{s}")?;
        let n = self.calls;
        self.calls += 1;
        if n == self.op["fail"].as_i64().unwrap() {
            Err(io::Error::other("injected synchronized callback failure"))
        } else {
            Ok(())
        }
    }
}
impl Observer for Callback<'_> {
    type Error = io::Error;
    fn updated(&mut self, data: &mut SyncedFloats, id: u8) -> io::Result<()> {
        self.event(&format!(
            "updated {id} {:08x} {} {}",
            f32_bits(data.get(id).unwrap()),
            data.is_dirty(),
            data.item_dirty(id).unwrap()
        ))?;
        if self.nested {
            return Ok(());
        }
        self.nested = true;
        let other = self.op["other"].as_u64().unwrap() as u8;
        let value = single(&self.op["other_value"]);
        let result = match self.op["hook"].as_str().unwrap() {
            "none" => Ok(()),
            "pack" => writeln!(
                self.out,
                "callback-pack {}",
                values(data.take_dirty().as_deref())
            ),
            "set" => data
                .set(
                    other,
                    value,
                    self.op["other_force"].as_bool().unwrap(),
                    self,
                )
                .map_err(|e| match e {
                    DataError::Observer(e) => e,
                    DataError::UnknownId(_) => panic!("hook uses registered ID"),
                }),
            "assign" => data
                .assign(&[FloatValue { id: other, value }], self)
                .map_err(|e| match e {
                    DataError::Observer(e) => e,
                    DataError::UnknownId(_) => panic!("hook uses registered ID"),
                }),
            _ => panic!("hook"),
        };
        self.nested = false;
        result
    }
    fn assigned(&mut self, _: &mut SyncedFloats, v: &[FloatValue]) -> io::Result<()> {
        self.event(&format!("assigned {}", values(Some(v))))
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).unwrap();
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let initial = parse(&f["initial"]);
        let mut ids = initial.iter().map(|v| v.id).collect::<Vec<_>>();
        ids.sort_unstable();
        let mut data = SyncedFloats::new(&initial)?;
        for op in f["ops"].as_array().unwrap() {
            let mut cb = Callback {
                out: &mut out,
                op,
                calls: 0,
                nested: false,
            };
            let id = op["id"].as_u64().unwrap() as u8;
            let result = match op["mode"].as_str().unwrap() {
                "set" => data.set(
                    id,
                    single(&op["value"]),
                    op["force"].as_bool().unwrap(),
                    &mut cb,
                ),
                "assign" => data.assign(&parse(&op["values"]), &mut cb),
                "get" => data.get(id).map_err(DataError::UnknownId).and_then(|v| {
                    writeln!(cb.out, "get {:08x}", f32_bits(v)).map_err(DataError::Observer)
                }),
                "pack" => writeln!(cb.out, "packed {}", values(data.take_dirty().as_deref()))
                    .map_err(DataError::Observer),
                "defaults" => {
                    writeln!(cb.out, "defaults {}", values(data.non_default().as_deref()))
                        .map_err(DataError::Observer)
                }
                _ => panic!("mode"),
            };
            match result {
                Ok(()) => writeln!(cb.out, "result ok")?,
                Err(DataError::Observer(_)) => writeln!(cb.out, "error observer")?,
                Err(DataError::UnknownId(_)) => writeln!(cb.out, "error id")?,
            }
            writeln!(cb.out, "dirty {}", data.is_dirty())?;
            for id in &ids {
                writeln!(
                    cb.out,
                    "state {id} {:08x} {} {}",
                    f32_bits(data.get(*id).unwrap()),
                    data.item_dirty(*id).unwrap(),
                    data.is_default(*id).unwrap()
                )?;
            }
            writeln!(cb.out, "calls {}", cb.calls)?;
        }
    }
    Ok(())
}
