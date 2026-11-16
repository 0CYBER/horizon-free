//! Loads, equips and saves entities in the format of `VanillaMobAttributeNbtOracle`
//! (tools/test_mob_attribute_nbt_parity.py): the attribute values and bases asked
//! for, and the saved `attributes` list as exact double bits.
//!
//! usage: mob_attribute_nbt_fixture RESOURCES CASES OUTPUT
use horizon_client::{
    entity_nbt,
    items::{ItemStack, Items},
    living_attributes::LivingAttributes,
    mobs::MobTypes,
};
use horizon_items::equipment::{Equipment, SLOT_NAMES};
use serde_json::Value;
use std::{fmt::Write as _, fs, path::Path, sync::Arc};
use {
    horizon_core::nbt::Tag,
    horizon_world::{attribute_map::ValueKind, attributes::entity::Catalog, loot::PackOverlay},
};

fn slot(name: &str) -> usize {
    SLOT_NAMES.iter().position(|n| *n == name).unwrap()
}

fn text(tag: Option<&Tag>) -> String {
    match tag {
        Some(Tag::String(s)) => s.to_string_lossy(),
        _ => "?".into(),
    }
}

fn bits(tag: Option<&Tag>) -> String {
    match tag {
        Some(Tag::Double(b)) => format!("{b:x}"),
        _ => "?".into(),
    }
}

/// `saved id base ... mod id amount operation ...` per attribute, sorted by id.
fn saved(tag: &Tag) -> String {
    let Some(Tag::List(list)) = tag.get("attributes") else {
        return String::new();
    };
    let mut rows: Vec<(String, String)> = list
        .iter()
        .map(|a| {
            let mut row = format!("base {}", bits(a.get("base")));
            if let Some(Tag::List(mods)) = a.get("modifiers") {
                for m in mods {
                    let _ = write!(
                        row,
                        " mod {} {} {}",
                        text(m.get("id")),
                        bits(m.get("amount")),
                        text(m.get("operation"))
                    );
                }
            }
            (text(a.get("id")), row)
        })
        .collect();
    rows.sort();
    rows.iter().fold(String::new(), |out, (id, row)| {
        format!("{out}\tsaved {id} {row}")
    })
}

fn stack(item: &Value) -> Option<ItemStack> {
    item.as_str()
        .filter(|s| !s.is_empty())
        .map(|s| ItemStack::new(s, 1))
}

pub fn main() -> horizon_client::Result<()> {
    let args: Vec<String> = crate::args().collect();
    let usage = "usage: mob_attribute_nbt_fixture RESOURCES CASES OUTPUT";
    let root = Path::new(args.get(1).ok_or(usage)?);
    let cases = fs::read_to_string(args.get(2).ok_or(usage)?)?;
    let defaults = horizon_world::item_defaults::read(root)?;
    let items = Items::catalog(root, &PackOverlay::NONE, &defaults)?;
    let types = MobTypes::load(root)?;
    let mobs: std::collections::HashMap<_, _> = types
        .types
        .iter()
        .map(|(k, v)| (k.clone(), Arc::new(v.clone())))
        .collect();
    let catalog = Arc::new(Catalog::from_json(&serde_json::from_slice(&fs::read(
        root.join("attributes.json"),
    )?)?)?);
    let mut out = String::new();
    for line in cases.lines().filter(|l| !l.trim().is_empty()) {
        let case: Value = serde_json::from_str(line)?;
        let name = case["name"].as_str().unwrap_or_default();
        let entity = case["entity"].as_str().unwrap_or_default();
        let (tag, _) = horizon_core::nbt::parse_snbt(case["snbt"].as_str().unwrap_or_default())?;
        let passes = [&case["equip"], &case["then"]];
        let mut row = name.to_owned();
        // Equips the slots a pass names, then runs the equipment pass on `live`.
        let run = |live: &mut LivingAttributes,
                   equipment: &mut Equipment,
                   seed: &dyn Fn() -> Vec<Tag>| {
            for pass in passes {
                for (slot_name, item) in pass.as_object().into_iter().flatten() {
                    equipment.set(slot(slot_name), stack(item));
                }
                live.collect_equipment_changes(&catalog, entity, equipment, &items, seed)
                    .unwrap();
            }
        };
        let (reads, saved_tag) = if entity == "minecraft:armor_stand" {
            let mut stand = entity_nbt::armor_stand_from_nbt(&tag, 1).ok_or("stand")?;
            let mut live = std::mem::take(&mut stand.live);
            run(&mut live, &mut stand.equipment, &Vec::new);
            stand.live = live;
            let reads: Vec<Option<(f64, f64)>> = case["read"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|n| read(&stand.live, &catalog, entity, n, Vec::new()))
                .collect();
            (reads, entity_nbt::armor_stand_to_nbt(&stand, [0; 4], None))
        } else {
            let mut mob = entity_nbt::mob_from_nbt(&tag, &mobs, 1, None).ok_or("mob")?;
            let mut live = std::mem::take(&mut mob.extra.live);
            let mut equipment = std::mem::take(&mut mob.extra.equipment);
            let seed = |mob: &horizon_client::mobs::Mob| -> Vec<Tag> {
                entity_nbt::attribute_entries(mob)
                    .into_iter()
                    .map(|(_, e)| e)
                    .collect()
            };
            run(&mut live, &mut equipment, &|| seed(&mob));
            mob.extra.live = live;
            mob.extra.equipment = equipment;
            let reads: Vec<Option<(f64, f64)>> = case["read"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|n| read(&mob.extra.live, &catalog, entity, n, seed(&mob)))
                .collect();
            (
                reads,
                entity_nbt::mob_to_nbt(&mob, [0; 4], None, None, |_| None),
            )
        };
        for (n, r) in case["read"].as_array().into_iter().flatten().zip(reads) {
            let n = n.as_str().unwrap_or_default();
            match r {
                Some((v, b)) => {
                    let _ = write!(row, "\tread {n} {:x} {:x}", v.to_bits(), b.to_bits());
                }
                None => {
                    let _ = write!(row, "\tread {n} none");
                }
            }
        }
        row.push_str(&saved(&saved_tag));
        out.push_str(&row);
        out.push('\n');
    }
    fs::write(args.get(3).ok_or(usage)?, out)?;
    Ok(())
}

fn read(
    live: &LivingAttributes,
    catalog: &Arc<Catalog>,
    entity: &str,
    name: &Value,
    seed: Vec<Tag>,
) -> Option<(f64, f64)> {
    let name = name.as_str()?;
    let value = live.read(catalog, entity, name, ValueKind::Computed, || seed.clone())?;
    let base = live.read(catalog, entity, name, ValueKind::Base, || seed)?;
    Some((value, base))
}
