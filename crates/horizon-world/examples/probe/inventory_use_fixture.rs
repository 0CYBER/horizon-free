use horizon_world::{
    component_patch::Prototype,
    inventory_use::{ActiveStack, InventoryUse, UseServices, UseView},
    item_stack_refs::{ItemStacks, StackRef},
    item_use_lifecycle::{self, UseState},
    living_use_flags,
    player_equipment::InteractionHand,
    player_inventory::{EntityEquipment, EquipmentSlot, PlayerInventory},
    synced_values::{DataValue, SyncedValues},
};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
type Stack = StackRef<usize, u8, bool>;
type State = ActiveStack<usize, u8, bool>;
type View<'a> = UseView<'a, usize, u8, bool>;
fn n(v: &Value, k: &str) -> i32 {
    v[k].as_i64().unwrap() as i32
}
struct Services {
    f: Value,
    out: io::BufWriter<fs::File>,
    calls: i32,
    changed: bool,
    items: [Stack; 3],
}
impl Services {
    fn config(&self) -> &Value {
        &self.f[if self.changed { "after" } else { "initial" }]
    }
    fn id(&self, s: &Stack) -> usize {
        self.items.iter().position(|v| v.same_reference(s)).unwrap()
    }
    fn reset(&self, view: View<'_>, state: &mut State) {
        let c = self.config();
        state.item = self.items[n(c, "item") as usize].clone();
        state.remaining = n(c, "remaining");
        state.has_recent_kinetic_enemies = c["kinetic"].as_bool().unwrap();
        for i in 0..3 {
            self.items[i].write().count = c["counts"][i].as_i64().unwrap() as i32;
        }
        view.inventory.clear();
        view.inventory.set_selected(n(c, "selected")).unwrap();
        for p in c["slots"].as_array().unwrap() {
            view.inventory
                .set(
                    p[0].as_i64().unwrap() as i32,
                    self.items[p[1].as_u64().unwrap() as usize].clone(),
                )
                .unwrap();
        }
    }
    fn all(&mut self, view: &View<'_>, state: &State) -> io::Result<()> {
        writeln!(
            self.out,
            "state {} {} {} {} {} {} {} {} {} {} {}",
            self.id(&state.item),
            state.remaining,
            state.has_recent_kinetic_enemies,
            living_use_flags::using_item(view.flags).unwrap(),
            match living_use_flags::used_hand(view.flags).unwrap() {
                InteractionHand::Main => 0,
                InteractionHand::Off => 1,
            },
            view.flags.get(8).unwrap(),
            view.flags.is_dirty(),
            view.flags.item_dirty(8).unwrap(),
            view.inventory.selected(),
            self.calls,
            self.changed
        )?;
        writeln!(
            self.out,
            "counts {} {} {}",
            self.items[0].read().count,
            self.items[1].read().count,
            self.items[2].read().count
        )?;
        let slots = (0..43)
            .map(|i| self.id(&view.inventory.get(i).unwrap()).to_string())
            .collect::<Vec<_>>()
            .join(",");
        writeln!(self.out, "slots {slots}")?;
        let eq = view
            .inventory
            .equipment()
            .entries()
            .iter()
            .map(|(slot, s)| format!("{}={}", *slot as usize, self.id(s)))
            .collect::<Vec<_>>()
            .join(",");
        writeln!(self.out, "equipment {eq}")?;
        Ok(())
    }
    fn event(&mut self, view: View<'_>, state: &mut State, name: &str) -> io::Result<()> {
        writeln!(self.out, "{name}")?;
        let index = self.calls;
        self.calls += 1;
        self.all(&view, state)?;
        if index == n(&self.f, "fail") {
            return Err(io::Error::other("injected callback failure"));
        }
        if index == n(&self.f, "mutate_at") {
            self.changed = true;
            self.reset(view, state);
        }
        Ok(())
    }
}
impl UseServices<usize, u8, bool> for Services {
    type Error = io::Error;
    type Level = usize;
    fn level(&mut self, view: View<'_>, state: &mut State) -> io::Result<usize> {
        self.event(view, state, "level")?;
        Ok(n(self.config(), "level") as usize)
    }
    fn is_client(&mut self, view: View<'_>, state: &mut State, level: usize) -> io::Result<bool> {
        self.event(view, state, &format!("client {level}"))?;
        Ok(self.config()["clients"][level].as_bool().unwrap())
    }
    fn remaining_ticks(&mut self, view: View<'_>, state: &mut State) -> io::Result<i32> {
        self.event(view, state, "remaining")?;
        Ok(n(self.config(), "virtual_remaining"))
    }
    fn item_tick(
        &mut self,
        view: View<'_>,
        state: &mut State,
        item: Stack,
        level: usize,
        remaining: i32,
    ) -> io::Result<()> {
        let kind = item.read().item();
        if kind != 2 {
            self.event(
                view,
                state,
                &format!("tick {kind} {} {level} {remaining}", self.id(&item)),
            )?;
        }
        Ok(())
    }
    fn use_on_release(
        &mut self,
        view: View<'_>,
        state: &mut State,
        item: Stack,
    ) -> io::Result<bool> {
        let kind = item.read().item();
        if kind == 2 {
            return Ok(false);
        }
        self.event(
            view,
            state,
            &format!("release-query {kind} {}", self.id(&item)),
        )?;
        Ok(self.config()["releases"][kind].as_bool().unwrap())
    }
    fn release_item(
        &mut self,
        view: View<'_>,
        state: &mut State,
        item: Stack,
        level: usize,
        remaining: i32,
    ) -> io::Result<()> {
        let kind = item.read().item();
        if kind != 2 {
            self.event(
                view,
                state,
                &format!("release {kind} {} {level} {remaining}", self.id(&item)),
            )?;
        }
        Ok(())
    }
    fn finish_item(
        &mut self,
        view: View<'_>,
        state: &mut State,
        item: Stack,
        level: usize,
    ) -> io::Result<Stack> {
        let kind = item.read().item();
        if kind == 2 {
            return Ok(item);
        }
        self.event(
            view,
            state,
            &format!("finish {kind} {} {level}", self.id(&item)),
        )?;
        Ok(self.items[self.config()["finishes"][kind].as_u64().unwrap() as usize].clone())
    }
    fn finish_vibration(
        &mut self,
        view: View<'_>,
        state: &mut State,
        item: Stack,
    ) -> io::Result<()> {
        if item.read().component(&0).copied().unwrap_or(false) {
            self.event(view, state, "vibration")?;
        }
        Ok(())
    }
    fn on_equip(
        &mut self,
        view: View<'_>,
        state: &mut State,
        slot: EquipmentSlot,
        old: Stack,
        new: Stack,
    ) -> io::Result<()> {
        self.event(
            view,
            state,
            &format!(
                "equip {} {} {}",
                slot as usize,
                self.id(&old),
                self.id(&new)
            ),
        )
    }
    fn on_synced(&mut self, view: View<'_>, state: &mut State, id: u8) -> io::Result<()> {
        self.event(view, state, &format!("synced {id}"))
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).unwrap();
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let stacks = ItemStacks::new(2);
        let items = [
            stacks.create(0, 1, Prototype::new([])),
            stacks.create(
                if f["shared_type"].as_bool().unwrap() {
                    0
                } else {
                    1
                },
                1,
                Prototype::new([]),
            ),
            stacks.empty(),
        ];
        for i in 0..2 {
            if f["vibrations"][i].as_bool().unwrap() {
                items[i].write().components.set(0, Some(true)).unwrap();
            }
        }
        let mut state = UseState {
            item: items[2].clone(),
            remaining: 0,
            has_recent_kinetic_enemies: false,
        };
        let inventory = PlayerInventory::new(stacks.clone(), EntityEquipment::new(stacks.empty()));
        let flags = SyncedValues::new(&[DataValue {
            id: 8,
            value: n(&f, "flags") as u8,
        }])?;
        let services = Services {
            f: f.clone(),
            out,
            calls: 0,
            changed: false,
            items,
        };
        let mut world = InventoryUse::new(inventory, flags, services).unwrap();
        world.services.reset(
            UseView {
                inventory: &mut world.inventory,
                flags: &mut world.flags,
            },
            &mut state,
        );
        world.services.all(
            &UseView {
                inventory: &mut world.inventory,
                flags: &mut world.flags,
            },
            &state,
        )?;
        let result = match f["mode"].as_str().unwrap() {
            "tick" => item_use_lifecycle::tick(&mut world, &mut state),
            "update" => {
                let item = world.services.items[n(&f, "argument") as usize].clone();
                item_use_lifecycle::update(&mut world, &mut state, item)
            }
            "complete" => item_use_lifecycle::complete(&mut world, &mut state),
            "release" => item_use_lifecycle::release(&mut world, &mut state),
            "stop" => item_use_lifecycle::stop(&mut world, &mut state),
            _ => panic!("mode"),
        };
        writeln!(
            world.services.out,
            "result {}",
            if result.is_ok() { "ok" } else { "error" }
        )?;
        world.services.all(
            &UseView {
                inventory: &mut world.inventory,
                flags: &mut world.flags,
            },
            &state,
        )?;
        let packed = world.flags.take_dirty().map_or("none".into(), |values| {
            values
                .iter()
                .map(|v| format!("{}:{}", v.id, v.value))
                .collect::<Vec<_>>()
                .join(",")
        });
        writeln!(world.services.out, "packed {packed}")?;
        world.services.all(
            &UseView {
                inventory: &mut world.inventory,
                flags: &mut world.flags,
            },
            &state,
        )?;
        out = world.services.out;
    }
    Ok(())
}
