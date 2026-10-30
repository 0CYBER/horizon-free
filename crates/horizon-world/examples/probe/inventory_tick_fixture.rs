use horizon_world::{
    component_patch::Prototype,
    inventory_tick::{self, InventoryTickWorld},
    item_stack_refs::{ItemStacks, StackRef},
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
    level: u8,
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
            self.level,
            self.calls
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
        writeln!(self.out, "equipment {entries}").unwrap();
        let s = self.snapshot(&self.empty.clone());
        writeln!(self.out, "empty {s}").unwrap();
    }
    fn event(&mut self, inv: &mut Inventory, name: &str) -> Result<(), ()> {
        writeln!(self.out, "{name}").unwrap();
        self.calls += 1;
        self.all(inv);
        if self.calls == n(&self.f, "fail_at") {
            return Err(());
        }
        if self.calls == n(&self.f, "mutate_at") {
            self.refs[n(&self.f, "mutate_ref") as usize].write().count = n(&self.f, "after_count");
            self.refs[n(&self.f, "pop_ref") as usize].write().pop_time = n(&self.f, "after_pop");
            if n(&self.f, "replace_slot") >= 0 {
                inv.set(
                    n(&self.f, "replace_slot"),
                    self.refs[n(&self.f, "replace_ref") as usize].clone(),
                )
                .unwrap();
            }
            if n(&self.f, "equipment_slot") >= 0 {
                inv.equipment().set(
                    EquipmentSlot::ALL[n(&self.f, "equipment_slot") as usize],
                    self.refs[n(&self.f, "equipment_ref") as usize].clone(),
                );
            }
            inv.set_selected(n(&self.f, "after_selected")).unwrap();
            self.level = n(&self.f, "after_level") as u8;
        }
        Ok(())
    }
}
impl InventoryTickWorld<u8, u8, i32> for World {
    type Error = ();
    type Level = u8;
    fn level(&mut self, inv: &mut Inventory) -> Result<u8, ()> {
        self.event(inv, "level")?;
        Ok(self.level)
    }
    fn is_server_level(&self, level: u8) -> bool {
        level == 1
    }
    fn item_tick(
        &mut self,
        inv: &mut Inventory,
        item: u8,
        stack: Stack,
        level: u8,
        slot: Option<EquipmentSlot>,
    ) -> Result<(), ()> {
        assert_eq!(level, 1);
        if item != 0 {
            let id = self.id(&stack);
            self.event(
                inv,
                &format!("tick {item} {id} {}", slot.map_or(-1, |s| s as i32)),
            )?;
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
            inv.equipment().set(
                EquipmentSlot::ALL[p[0].as_u64().unwrap() as usize],
                refs[p[1].as_u64().unwrap() as usize].clone(),
            );
        }
        let mut w = World {
            out,
            f: f.clone(),
            refs,
            ids: vec![empty.clone()],
            empty,
            level: n(&f, "level") as u8,
            calls: 0,
        };
        w.all(&inv);
        let result = match f["mode"].as_str().unwrap() {
            "inventory" => inventory_tick::tick_inventory(&mut w, &mut inv),
            "equipment" => inventory_tick::tick_equipment(&mut w, &mut inv),
            "both" => inventory_tick::tick_inventory(&mut w, &mut inv)
                .and_then(|()| inventory_tick::tick_equipment(&mut w, &mut inv)),
            "stack" => {
                let stack = w.refs[n(&f, "argument") as usize].clone();
                inventory_tick::tick_stack(
                    &mut w,
                    &mut inv,
                    stack,
                    n(&f, "argument_level") as u8,
                    if n(&f, "slot") < 0 {
                        None
                    } else {
                        Some(EquipmentSlot::ALL[n(&f, "slot") as usize])
                    },
                )
            }
            _ => panic!("mode"),
        };
        writeln!(
            w.out,
            "result {}",
            if result.is_ok() { "ok" } else { "error" }
        )?;
        w.all(&inv);
        out = w.out;
    }
    Ok(())
}
