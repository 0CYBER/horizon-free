use horizon_world::{
    component_patch::{Patch, Prototype},
    item_stack_refs::{ItemStacks, StackRef},
    player_inventory::{EntityEquipment, EquipmentSlot, InventoryRules, PlayerInventory},
};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
type Stack = StackRef<u8, usize, i32>;
type Inventory = PlayerInventory<u8, usize, i32>;
struct Rules;
impl InventoryRules<u8, usize, i32> for Rules {
    fn is_enchanted(&self, s: &Stack) -> bool {
        s.read().component(&4).is_some_and(|n| *n != 0)
    }
    fn is_damaged(&self, s: &Stack) -> bool {
        let s = s.read();
        s.component(&2).is_some()
            && s.component(&3).is_none()
            && s.component(&1).is_some()
            && s.component(&1)
                .copied()
                .unwrap_or(0)
                .max(0)
                .min(s.component(&2).copied().unwrap_or(0))
                > 0
    }
    fn has_custom_name(&self, s: &Stack) -> bool {
        s.read().component(&5).is_some()
    }
}
fn n(v: &Value, k: &str) -> i32 {
    v[k].as_i64().unwrap() as i32
}
fn opt(v: Option<i32>) -> String {
    v.map_or("none".into(), |n| n.to_string())
}
struct Output {
    out: io::BufWriter<fs::File>,
    ids: Vec<Stack>,
}
impl Output {
    fn snapshot(&mut self, s: &Stack) -> String {
        let id = self
            .ids
            .iter()
            .position(|v| v.same_reference(s))
            .unwrap_or_else(|| {
                self.ids.push(s.clone());
                self.ids.len() - 1
            });
        let damaged = Rules.is_damaged(s);
        let enchanted = Rules.is_enchanted(s);
        let s = s.read();
        format!(
            "{}:{}:{}:{}:{}:{}:{}:{}:{}:{}",
            id,
            s.item(),
            s.count,
            s.count(),
            s.pop_time,
            s.is_empty(),
            damaged,
            enchanted,
            (0..6)
                .map(|k| opt(s.component(&k).copied()))
                .collect::<Vec<_>>()
                .join(","),
            s.components
                .as_patch()
                .entries()
                .iter()
                .map(|(k, v)| format!("{k}={}", opt(*v)))
                .collect::<Vec<_>>()
                .join(",")
        )
    }
    fn all(&mut self, refs: &[Stack], inv: &[Inventory; 2], empty: &Stack) {
        for (i, s) in refs.iter().enumerate() {
            let s = self.snapshot(s);
            writeln!(self.out, "ref {i} {s}").unwrap();
        }
        for (i, c) in inv.iter().enumerate() {
            writeln!(
                self.out,
                "inventory {} {} {} {} {} {} {} {} {}",
                i,
                c.selected(),
                c.times_changed,
                c.free_slot(),
                c.suitable_hotbar(&Rules),
                c.is_empty(),
                c.count_item(0),
                c.count_item(1),
                c.count_item(2)
            )
            .unwrap();
            let slots = (0..43)
                .map(|j| self.snapshot(&c.get(j).unwrap()))
                .collect::<Vec<_>>()
                .join(" | ");
            writeln!(self.out, "slots {slots}").unwrap();
            let eq = c
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
            writeln!(self.out, "equipment {eq}").unwrap();
            for (j, s) in refs.iter().enumerate() {
                writeln!(
                    self.out,
                    "query {} {} {} {}",
                    j,
                    c.contains(s),
                    c.find_matching(s),
                    c.find_crafting(1, s, &Rules)
                )
                .unwrap();
            }
        }
        let e = self.snapshot(empty);
        writeln!(self.out, "empty {e}").unwrap();
    }
}
struct World {
    report: Output,
    refs: Vec<Stack>,
    op: Value,
    calls: i32,
}
impl World {
    fn event(&mut self, c: &mut Inventory, name: &str) -> Result<(), ()> {
        self.calls += 1;
        writeln!(self.report.out, "event {name} {}", self.calls).unwrap();
        writeln!(
            self.report.out,
            "active {} {}",
            c.selected(),
            c.times_changed
        )
        .unwrap();
        for i in 0..self.refs.len() {
            let s = self.report.snapshot(&self.refs[i]);
            writeln!(self.report.out, "active-ref {i} {s}").unwrap();
        }
        let slots = (0..43)
            .map(|j| self.report.snapshot(&c.get(j).unwrap()))
            .collect::<Vec<_>>()
            .join(" | ");
        writeln!(self.report.out, "active-slots {slots}").unwrap();
        if self.calls == n(&self.op, "fail_at") {
            return Err(());
        }
        if self.calls == n(&self.op, "mutate_at") {
            let mut s = self.refs[n(&self.op, "mutate_ref") as usize].write();
            s.count = n(&self.op, "after_count");
            s.components
                .set(0, Some(n(&self.op, "after_max")))
                .map_err(|_| ())?;
            drop(s);
            if n(&self.op, "mutate_slot") >= 0 {
                c.set(
                    n(&self.op, "mutate_slot"),
                    self.refs[n(&self.op, "replace_ref") as usize].clone(),
                )
                .map_err(|_| ())?;
            }
            if n(&self.op, "after_selected") >= 0 {
                c.set_selected(n(&self.op, "after_selected"))
                    .map_err(|_| ())?;
            }
        }
        Ok(())
    }
}
impl InventoryRules<u8, usize, i32> for World {
    fn is_enchanted(&self, s: &Stack) -> bool {
        Rules.is_enchanted(s)
    }
    fn is_damaged(&self, s: &Stack) -> bool {
        Rules.is_damaged(s)
    }
    fn has_custom_name(&self, s: &Stack) -> bool {
        Rules.has_custom_name(s)
    }
}
impl horizon_world::inventory_pickup::PickupHooks<u8, usize, i32> for World {
    type Error = ();
    fn item_max_stack_size(&self, s: &Stack) -> i32 {
        s.read().component(&0).copied().unwrap_or(1)
    }
    fn max_stack_size(&mut self, c: &mut Inventory) -> Result<i32, ()> {
        self.event(c, "max")?;
        Ok(n(&self.op, "max"))
    }
    fn infinite_materials(&mut self, c: &mut Inventory) -> Result<bool, ()> {
        self.event(c, "infinite")?;
        Ok(self.op["infinite"].as_bool().unwrap())
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
        let prototypes: Vec<_> = f["prototypes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| {
                Prototype::new(p.as_array().unwrap().iter().map(|v| {
                    (
                        v[0].as_u64().unwrap() as usize,
                        v[1].as_i64().unwrap() as i32,
                    )
                }))
            })
            .collect();
        let refs: Vec<_> = f["stacks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| {
                let item = n(v, "item");
                if item < 0 {
                    empty.clone()
                } else {
                    let s =
                        stacks.create(item as u8, n(v, "count"), prototypes[item as usize].clone());
                    s.write().pop_time = n(v, "pop");
                    s.write().components.restore_patch(&Patch::new(
                        v["patch"].as_array().unwrap().iter().map(|v| {
                            (
                                v[0].as_u64().unwrap() as usize,
                                v[1].as_i64().map(|n| n as i32),
                            )
                        }),
                    ));
                    s
                }
            })
            .collect();
        let equipment = EntityEquipment::new(empty.clone());
        let other = if f["shared_equipment"].as_bool().unwrap() {
            equipment.clone()
        } else {
            EntityEquipment::new(empty.clone())
        };
        let mut inv = [
            Inventory::new(stacks.clone(), equipment),
            Inventory::new(stacks.clone(), other),
        ];
        let order: [EquipmentSlot; 7] = std::array::from_fn(|i| {
            EquipmentSlot::from_inventory_slot(
                36 + f["equipment_order"][i].as_i64().unwrap() as i32,
            )
            .unwrap()
        });
        for (i, c) in inv.iter_mut().enumerate() {
            let initial = &f["inventories"][i];
            c.set_selected(n(initial, "selected")).unwrap();
            c.times_changed = n(initial, "changed");
            for p in initial["slots"].as_array().unwrap() {
                c.set(
                    p[0].as_i64().unwrap() as i32,
                    refs[p[1].as_u64().unwrap() as usize].clone(),
                )
                .unwrap();
            }
            if n(initial, "mainhand") >= 0 {
                c.equipment().set(
                    EquipmentSlot::MainHand,
                    refs[n(initial, "mainhand") as usize].clone(),
                );
            }
        }
        let mut w = World {
            report: Output {
                out,
                ids: vec![empty.clone()],
            },
            refs,
            op: Value::Null,
            calls: 0,
        };
        w.report.all(&w.refs, &inv, &empty);
        for op in f["ops"].as_array().unwrap() {
            w.op = op.clone();
            w.calls = 0;
            let target = n(op, "inventory") as usize;
            let a = n(op, "a") as usize;
            let b = n(op, "b") as usize;
            let slot = n(op, "slot");
            let amount = n(op, "amount");
            let result = (|| -> Result<(), ()> {
                match op["kind"].as_str().unwrap() {
                    "add" => {
                        let input = w.refs[a].clone();
                        let added = horizon_world::inventory_pickup::add(
                            &mut inv[target],
                            slot,
                            &input,
                            &mut w,
                        )
                        .map_err(|_| ())?;
                        writeln!(w.report.out, "added {added}").unwrap();
                    }
                    "space" => {
                        let input = w.refs[a].clone();
                        let space = horizon_world::inventory_pickup::slot_with_space(
                            &mut inv[target],
                            &input,
                            &mut w,
                        )
                        .map_err(|_| ())?;
                        writeln!(w.report.out, "space {space}").unwrap();
                    }
                    "get" => w.refs[b] = inv[target].get(slot).map_err(|_| ())?,
                    "set" => inv[target].set(slot, w.refs[a].clone()).map_err(|_| ())?,
                    "remove" => w.refs[b] = inv[target].remove(slot, amount),
                    "no-update" => {
                        w.refs[b] = inv[target].remove_no_update(slot).map_err(|_| ())?
                    }
                    "remove-ref" => inv[target].remove_reference(&w.refs[a], &order),
                    "select" => inv[target].set_selected(slot).map_err(|_| ())?,
                    "selected" => w.refs[b] = inv[target].selected_item(),
                    "set-selected" => w.refs[b] = inv[target].set_selected_item(w.refs[a].clone()),
                    "remove-selected" => {
                        w.refs[b] = inv[target].remove_selected(op["all"].as_bool().unwrap())
                    }
                    "pick" => inv[target].pick_slot(slot, &Rules).map_err(|_| ())?,
                    "add-pick" => inv[target].add_and_pick(w.refs[a].clone(), &Rules),
                    "clear" => inv[target].clear(),
                    "changed" => inv[target].changed(),
                    "replace" => {
                        let other = n(op, "other") as usize;
                        if other == target {
                            inv[target].replace_self();
                        } else {
                            let (first, second) = inv.split_at_mut(1);
                            if target == 0 {
                                first[0].replace_with(&second[0]);
                            } else {
                                second[0].replace_with(&first[0]);
                            }
                        }
                    }
                    "equip" => {
                        w.refs[b] = inv[target].equipment().set(
                            EquipmentSlot::ALL[n(op, "equipment") as usize],
                            w.refs[a].clone(),
                        )
                    }
                    "get-equip" => {
                        w.refs[b] = inv[target]
                            .equipment()
                            .get(EquipmentSlot::ALL[n(op, "equipment") as usize])
                    }
                    "alias" => w.refs[b] = w.refs[a].clone(),
                    "copy" => w.refs[b] = stacks.copy(&w.refs[a]),
                    "count" => w.refs[a].write().count = amount,
                    "component" => {
                        w.refs[a]
                            .write()
                            .components
                            .set(
                                n(op, "key") as usize,
                                op["value"].as_i64().map(|n| n as i32),
                            )
                            .map_err(|_| ())?;
                    }
                    _ => panic!("operation"),
                }
                Ok(())
            })();
            writeln!(
                w.report.out,
                "result {} {}",
                if result.is_ok() { "ok" } else { "error" },
                w.calls
            )?;
            w.report.all(&w.refs, &inv, &empty);
        }
        out = w.report.out;
    }
    Ok(())
}
