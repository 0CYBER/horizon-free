use horizon_world::{
    component_patch::Prototype,
    item_stack_refs::{ItemStacks, StackRef},
    player_equipment::{self, EquipWorld, InteractionHand},
    player_inventory::{EntityEquipment, EquipmentSlot, PlayerInventory},
};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
type Stack = StackRef<u8, u8, i32>;
type Inventory = PlayerInventory<u8, u8, i32>;
fn n(v: &Value, k: &str) -> i32 {
    v[k].as_i64().unwrap() as i32
}
struct World {
    out: io::BufWriter<fs::File>,
    f: Value,
    refs: Vec<Stack>,
    ids: Vec<Stack>,
    empty: Stack,
    calls: i32,
}
impl World {
    fn id(&mut self, s: &Stack) -> usize {
        self.ids
            .iter()
            .position(|v| v.same_reference(s))
            .unwrap_or_else(|| {
                self.ids.push(s.clone());
                self.ids.len() - 1
            })
    }
    fn snapshot(&mut self, s: &Stack) -> String {
        let id = self.id(s);
        let s = s.read();
        format!(
            "{}:{}:{}:{}:{}:{}",
            id,
            s.item(),
            s.count,
            s.count(),
            s.pop_time,
            s.is_empty()
        )
    }
    fn all(&mut self, inv: &Inventory) {
        for i in 0..self.refs.len() {
            let s = self.snapshot(&self.refs[i].clone());
            writeln!(self.out, "ref {i} {s}").unwrap();
        }
        writeln!(
            self.out,
            "state {} {} {}",
            inv.selected(),
            self.calls,
            player_equipment::is_empty(inv)
        )
        .unwrap();
        let slots = (0..43)
            .map(|i| self.snapshot(&inv.get(i).unwrap()))
            .collect::<Vec<_>>()
            .join(" | ");
        writeln!(self.out, "slots {slots}").unwrap();
        let entries = inv
            .equipment()
            .entries()
            .into_iter()
            .map(|(s, v)| {
                format!(
                    "{}={}",
                    match s {
                        EquipmentSlot::MainHand => "MAINHAND",
                        EquipmentSlot::OffHand => "OFFHAND",
                        EquipmentSlot::Feet => "FEET",
                        EquipmentSlot::Legs => "LEGS",
                        EquipmentSlot::Chest => "CHEST",
                        EquipmentSlot::Head => "HEAD",
                        EquipmentSlot::Body => "BODY",
                        EquipmentSlot::Saddle => "SADDLE",
                    },
                    self.snapshot(&v)
                )
            })
            .collect::<Vec<_>>()
            .join(" | ");
        writeln!(self.out, "raw-equipment {entries}").unwrap();
        let equipped = EquipmentSlot::ALL
            .into_iter()
            .map(|slot| self.snapshot(&player_equipment::get(inv, slot)))
            .collect::<Vec<_>>()
            .join(" | ");
        writeln!(self.out, "equipped {equipped}").unwrap();
        let main = self.snapshot(&player_equipment::get_hand(inv, InteractionHand::Main));
        let off = self.snapshot(&player_equipment::get_hand(inv, InteractionHand::Off));
        writeln!(self.out, "hands {main} | {off}").unwrap();
        let s = self.snapshot(&self.empty.clone());
        writeln!(self.out, "empty {s}").unwrap();
    }
}
impl EquipWorld<u8, u8, i32> for World {
    type Error = ();
    fn on_equip(
        &mut self,
        inv: &mut Inventory,
        slot: EquipmentSlot,
        old: Stack,
        next: Stack,
    ) -> Result<(), ()> {
        self.calls += 1;
        let old = self.id(&old);
        let next = self.id(&next);
        writeln!(self.out, "equip {} {old} {next}", slot as usize).unwrap();
        self.all(inv);
        if self.f["fail"].as_bool().unwrap() {
            return Err(());
        }
        if self.f["mutate"].as_bool().unwrap() {
            self.refs[n(&self.f, "mutate_ref") as usize].write().count = n(&self.f, "after_count");
            inv.set_selected(n(&self.f, "after_selected"))
                .map_err(|_| ())?;
            inv.set(
                n(&self.f, "replace_slot"),
                self.refs[n(&self.f, "replace_ref") as usize].clone(),
            )
            .map_err(|_| ())?;
        }
        Ok(())
    }
}

pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).unwrap();
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let stacks = ItemStacks::new(0);
        let empty = stacks.empty();
        let refs: Vec<_> = f["stacks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| {
                let item = n(v, "item");
                if item < 0 {
                    empty.clone()
                } else {
                    let s = stacks.create(item as u8, n(v, "count"), Prototype::new([]));
                    s.write().pop_time = n(v, "pop");
                    s
                }
            })
            .collect();
        let equipment = EntityEquipment::new(empty.clone());
        let mut inv = Inventory::new(stacks, equipment);
        inv.set_selected(n(&f, "selected")).unwrap();
        for p in f["slots"].as_array().unwrap() {
            inv.set(
                p[0].as_i64().unwrap() as i32,
                refs[p[1].as_u64().unwrap() as usize].clone(),
            )
            .unwrap();
        }
        for p in f["equipment"].as_array().unwrap() {
            player_equipment::set(
                &mut inv,
                EquipmentSlot::ALL[p[0].as_u64().unwrap() as usize],
                refs[p[1].as_u64().unwrap() as usize].clone(),
            );
        }
        for p in f["raw_entries"].as_array().unwrap() {
            inv.equipment().set(
                EquipmentSlot::ALL[p[0].as_u64().unwrap() as usize],
                refs[p[1].as_u64().unwrap() as usize].clone(),
            );
        }
        let mut w = World {
            out,
            f: Value::Null,
            refs,
            ids: vec![empty.clone()],
            empty,
            calls: 0,
        };
        w.all(&inv);
        for op in f["ops"].as_array().unwrap() {
            w.f = op.clone();
            w.calls = 0;
            let a = n(op, "a") as usize;
            let b = n(op, "b") as usize;
            let slot = n(op, "slot");
            let result = (|| -> Result<(), ()> {
                match op["kind"].as_str().unwrap() {
                    "get-hand" => {
                        w.refs[b] = player_equipment::get_hand(
                            &inv,
                            if n(op, "hand") == 0 {
                                InteractionHand::Main
                            } else {
                                InteractionHand::Off
                            },
                        )
                    }
                    "set-hand" => {
                        let s = w.refs[a].clone();
                        player_equipment::set_hand(
                            &mut w,
                            &mut inv,
                            if n(op, "hand") == 0 {
                                InteractionHand::Main
                            } else {
                                InteractionHand::Off
                            },
                            s,
                        )?;
                    }
                    "get-item" => {
                        w.refs[b] = player_equipment::get(
                            &inv,
                            EquipmentSlot::ALL[n(op, "equipment") as usize],
                        )
                    }
                    "set-item" => {
                        let s = w.refs[a].clone();
                        player_equipment::set_item_slot(
                            &mut w,
                            &mut inv,
                            EquipmentSlot::ALL[n(op, "equipment") as usize],
                            s,
                        )?;
                    }
                    "set-equipment" => {
                        w.refs[b] = player_equipment::set(
                            &mut inv,
                            EquipmentSlot::ALL[n(op, "equipment") as usize],
                            w.refs[a].clone(),
                        )
                    }
                    "clear-equipment" => inv.equipment().clear(),
                    "clear" => inv.clear(),
                    "select" => inv.set_selected(slot).map_err(|_| ())?,
                    "get" => w.refs[b] = inv.get(slot).map_err(|_| ())?,
                    "set" => inv.set(slot, w.refs[a].clone()).map_err(|_| ())?,
                    "count" => w.refs[a].write().count = n(op, "amount"),
                    "alias" => w.refs[b] = w.refs[a].clone(),
                    "copy" => {
                        let copied = w.refs[a].read().copy();
                        w.refs[b] = match copied {
                            horizon_world::item_stack_storage::CopyResult::Empty => w.empty.clone(),
                            horizon_world::item_stack_storage::CopyResult::Stack(data) => {
                                Stack::new(data)
                            }
                        };
                    }
                    _ => panic!("operation"),
                }
                Ok(())
            })();
            writeln!(
                w.out,
                "result {} {}",
                if result.is_ok() { "ok" } else { "error" },
                w.calls
            )?;
            w.all(&inv);
        }
        out = w.out;
    }
    Ok(())
}
