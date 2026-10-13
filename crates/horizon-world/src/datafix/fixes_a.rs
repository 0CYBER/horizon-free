//! Fixes of 1.20.2 to 1.20.6 (keys 3564 to 3833).
use super::dynamic::{
    as_double, as_float, as_int, as_number, as_str, as_string, empty_map, int_list, is_list,
    namespaced, remove, rename, rename_and_fix, set, stream, string,
};
use super::json;
use super::schema::{Key, Ty, key};
use super::{Cx, FixFn, id_is, items, renamer, reread};
use crate::nbt::Tag;

pub(super) fn register(out: &mut Vec<(Key, FixFn)>) {
    out.push((key(3564, 0), |cx| {
        drop_invalid_sign_data(cx, "minecraft:sign")
    }));
    out.push((key(3564, 1), |cx| {
        drop_invalid_sign_data(cx, "minecraft:hanging_sign")
    }));
    out.push((key(3565, 0), random_sequence_settings));
    out.push((key(3566, 0), scoreboard_display_slots));
    out.push((key(3568, 0), mob_effect_ids));
    out.push((key(3683, 0), primed_tnt_block_state));
    out.push((key(3685, 0), projectile_stored_item));
    out.push((key(3692, 0), |cx| {
        block_rename(cx, &[("minecraft:grass", "minecraft:short_grass")])
    }));
    out.push((key(3692, 0), |cx| {
        cx.rename(
            Ty::ItemName,
            renamer(&[("minecraft:grass", "minecraft:short_grass")]),
        )
    }));
    out.push((key(3800, 0), |cx| {
        cx.rename(
            Ty::ItemName,
            renamer(&[("minecraft:scute", "minecraft:turtle_scute")]),
        )
    }));
    out.push((key(3803, 0), rename_sweeping));
    out.push((key(3807, 1), map_banner_block_pos));
    out.push((key(3808, 0), |cx| {
        horse_body_armor(cx, "minecraft:horse", "ArmorItem", true)
    }));
    out.push((key(3808, 1), |cx| {
        horse_body_armor(cx, "minecraft:llama", "DecorItem", false)
    }));
    out.push((key(3808, 2), |cx| {
        horse_body_armor(cx, "minecraft:trader_llama", "DecorItem", false)
    }));
    out.push((key(3809, 0), chested_horse_slots));
    out.push((key(3812, 0), wolf_health));
    out.push((key(3813, 0), block_pos_format));
    out.push((key(3814, 0), |cx| {
        attributes_rename_legacy(cx, &|id| {
            (namespaced(id) == "minecraft:horse.jump_strength")
                .then_some("minecraft:generic.jump_strength")
        })
    }));
    out.push((key(3818, 0), beehive_fields));
    out.push((key(3818, 0), empty_item_in_hotbar));
    out.push((key(3818, 1), banner_patterns));
    out.push((key(3818, 2), tipped_arrow_potion));
    out.push((key(3818, 4), particle_unflattening));
    out.push((key(3818, 5), |cx| {
        cx.each(Ty::ItemStack, items::componentize)
    }));
    out.push((key(3818, 6), area_effect_cloud_potion));
    out.push((key(3820, 0), player_head_profile));
    out.push((key(3820, 0), lodestone_compass_component));
    out.push((key(3825, 0), item_custom_name_to_item_name));
    out.push((key(3825, 0), banner_custom_name_to_item_name));
    out.push((key(3825, 0), trial_spawner_config));
    out.push((key(3828, 0), empty_item_in_villager_trade));
    out.push((key(3833, 0), remove_empty_brushable_item));
}

// ------------------------------------------------------------------ 3564

/// `BlockEntitySignDoubleSidedEditableTextFix.FIELDS_TO_DROP`.
pub(super) const SIGN_FIELDS_TO_DROP: [&str; 10] = [
    "Text1",
    "Text2",
    "Text3",
    "Text4",
    "FilteredText1",
    "FilteredText2",
    "FilteredText3",
    "FilteredText4",
    "Color",
    "GlowingText",
];

fn drop_invalid_sign_data(cx: &mut Cx, id: &str) {
    cx.block_entity(id, |be| {
        if super::dynamic::as_bool(be.get("_filtered_correct"), false) {
            remove(be, "_filtered_correct");
            return;
        }
        for side in ["front_text", "back_text"] {
            if let Some(t) = be.get_mut(side) {
                fix_sign_text(t);
            }
        }
        for f in SIGN_FIELDS_TO_DROP {
            remove(be, f);
        }
    });
}

fn fix_sign_text(t: &mut Tag) {
    if !is_list(t.get("filtered_messages")) {
        return;
    }
    let filtered = stream(t.get("filtered_messages"));
    let lines = stream(t.get("messages"));
    let empty = Tag::String(crate::nbt::JavaString(json::text_component_json(&[])));
    let new: Vec<Tag> = filtered
        .into_iter()
        .enumerate()
        .map(|(i, line)| {
            let fallback = lines.get(i).cloned().unwrap_or_else(|| empty.clone());
            if line == empty { fallback } else { line }
        })
        .collect();
    if new == lines {
        remove(t, "filtered_messages");
    } else {
        set(t, "filtered_messages", Tag::List(new));
    }
}

// ------------------------------------------------------------------ 3565, 3566

fn random_sequence_settings(cx: &mut Cx) {
    cx.each(Ty::SavedRandomSequences, |v| {
        if let Some(data) = remove(v, "data") {
            set(v, "data", Tag::compound([("sequences", data)]));
        }
    });
}

const SLOT_RENAMES: [(&str, &str); 19] = [
    ("slot_0", "list"),
    ("slot_1", "sidebar"),
    ("slot_2", "below_name"),
    ("slot_3", "sidebar.team.black"),
    ("slot_4", "sidebar.team.dark_blue"),
    ("slot_5", "sidebar.team.dark_green"),
    ("slot_6", "sidebar.team.dark_aqua"),
    ("slot_7", "sidebar.team.dark_red"),
    ("slot_8", "sidebar.team.dark_purple"),
    ("slot_9", "sidebar.team.gold"),
    ("slot_10", "sidebar.team.gray"),
    ("slot_11", "sidebar.team.dark_gray"),
    ("slot_12", "sidebar.team.blue"),
    ("slot_13", "sidebar.team.green"),
    ("slot_14", "sidebar.team.aqua"),
    ("slot_15", "sidebar.team.red"),
    ("slot_16", "sidebar.team.light_purple"),
    ("slot_17", "sidebar.team.yellow"),
    ("slot_18", "sidebar.team.white"),
];

fn scoreboard_display_slots(cx: &mut Cx) {
    cx.each(Ty::SavedScoreboard, |v| {
        if let Some(data) = v.get_mut("data")
            && let Some(Tag::Compound(slots)) = data.get_mut("DisplaySlots")
        {
            let old = std::mem::take(slots);
            for (k, value) in old {
                let name = k.to_string_lossy();
                let k = SLOT_RENAMES
                    .iter()
                    .find(|(o, _)| *o == name)
                    .map_or(k, |(_, n)| (*n).into());
                slots.insert(k, value);
            }
        }
    });
}

// ------------------------------------------------------------------ 3568

const EFFECT_IDS: [&str; 33] = [
    "minecraft:speed",
    "minecraft:slowness",
    "minecraft:haste",
    "minecraft:mining_fatigue",
    "minecraft:strength",
    "minecraft:instant_health",
    "minecraft:instant_damage",
    "minecraft:jump_boost",
    "minecraft:nausea",
    "minecraft:regeneration",
    "minecraft:resistance",
    "minecraft:fire_resistance",
    "minecraft:water_breathing",
    "minecraft:invisibility",
    "minecraft:blindness",
    "minecraft:night_vision",
    "minecraft:hunger",
    "minecraft:weakness",
    "minecraft:poison",
    "minecraft:wither",
    "minecraft:health_boost",
    "minecraft:absorption",
    "minecraft:saturation",
    "minecraft:glowing",
    "minecraft:levitation",
    "minecraft:luck",
    "minecraft:unluck",
    "minecraft:slow_falling",
    "minecraft:conduit_power",
    "minecraft:dolphins_grace",
    "minecraft:bad_omen",
    "minecraft:hero_of_the_village",
    "minecraft:darkness",
];

/// `getAndConvertMobEffectId`.
fn effect_id(v: &Tag, field: &str) -> Option<Tag> {
    let n = as_number(v.get(field)?)?.int_value();
    (1..=33)
        .contains(&n)
        .then(|| string(EFFECT_IDS[(n - 1) as usize]))
}
/// `replaceField(old, new, value)`.
fn replace_field(v: &mut Tag, old: &str, new: &str, value: Option<Tag>) {
    remove(v, old);
    if let Some(value) = value {
        set(v, new, value);
    }
}
fn update_effect_id_field(input: &Tag, old: &str, output: &mut Tag, new: &str) {
    let mapped = effect_id(input, old);
    replace_field(output, old, new, mapped);
}
fn update_effect_instance(mut v: Tag) -> Tag {
    let input = v.clone();
    update_effect_id_field(&input, "Id", &mut v, "id");
    rename(&mut v, "Ambient", "ambient");
    rename(&mut v, "Amplifier", "amplifier");
    rename(&mut v, "Duration", "duration");
    rename(&mut v, "ShowParticles", "show_particles");
    rename(&mut v, "ShowIcon", "show_icon");
    let hidden = v.get("HiddenEffect").cloned().map(update_effect_instance);
    replace_field(&mut v, "HiddenEffect", "hidden_effect", hidden);
    v
}
fn update_effect_list(v: &mut Tag, old: &str, new: &str) {
    let value = is_list(v.get(old)).then(|| {
        Tag::List(
            stream(v.get(old))
                .into_iter()
                .map(update_effect_instance)
                .collect(),
        )
    });
    replace_field(v, old, new, value);
}
fn update_stew_entry(input: &Tag, mut output: Tag) -> Tag {
    update_effect_id_field(input, "EffectId", &mut output, "id");
    let duration = input.get("EffectDuration").cloned();
    replace_field(&mut output, "EffectDuration", "duration", duration);
    output
}

fn mob_effect_ids(cx: &mut Cx) {
    cx.block_entity("minecraft:beacon", |be| {
        let input = be.clone();
        update_effect_id_field(&input, "Primary", be, "primary_effect");
        let input = be.clone();
        update_effect_id_field(&input, "Secondary", be, "secondary_effect");
    });
    cx.each(Ty::Entity, |e| {
        if id_is(e, "minecraft:mooshroom") {
            let initial = empty_map();
            let entry = update_stew_entry(e, initial.clone());
            if entry != initial {
                set(e, "stew_effects", Tag::List(vec![entry]));
            }
            remove(e, "EffectId");
            remove(e, "EffectDuration");
        }
        if id_is(e, "minecraft:arrow") {
            update_effect_list(e, "CustomPotionEffects", "custom_potion_effects");
        }
        if id_is(e, "minecraft:area_effect_cloud") {
            update_effect_list(e, "Effects", "effects");
        }
        update_effect_list(e, "ActiveEffects", "active_effects");
    });
    cx.each(Ty::Player, |p| {
        update_effect_list(p, "ActiveEffects", "active_effects")
    });
    cx.each(Ty::ItemStack, |s| {
        let Some(id) = as_str(s.get("id")).map(|i| namespaced(&i)) else {
            return;
        };
        let Some(tag) = s.get_mut("tag") else {
            return;
        };
        if id == "minecraft:suspicious_stew" {
            let value = is_list(tag.get("Effects")).then(|| {
                Tag::List(
                    stream(tag.get("Effects"))
                        .into_iter()
                        .map(|e| update_stew_entry(&e.clone(), e))
                        .collect(),
                )
            });
            replace_field(tag, "Effects", "effects", value);
        } else if matches!(
            id.as_str(),
            "minecraft:potion"
                | "minecraft:splash_potion"
                | "minecraft:lingering_potion"
                | "minecraft:tipped_arrow"
        ) {
            update_effect_list(tag, "CustomPotionEffects", "custom_potion_effects");
        }
    });
}

// ------------------------------------------------------------------ 3683, 3685

fn primed_tnt_block_state(cx: &mut Cx) {
    let k = cx.k;
    cx.entity("minecraft:tnt", |e| {
        set(
            e,
            "block_state",
            Tag::compound([("Name", string("minecraft:tnt"))]),
        );
        if let Some(fuse) = e.get("Fuse").cloned() {
            set(e, "fuse", fuse);
        }
        reread(k, e);
    });
}

fn projectile_stored_item(cx: &mut Cx) {
    let k = cx.k;
    cx.each(Ty::Entity, |e| {
        let item = if id_is(e, "minecraft:arrow") {
            let potion = as_string(e.get("Potion"), "minecraft:empty");
            if potion == "minecraft:empty" {
                "minecraft:arrow"
            } else {
                "minecraft:tipped_arrow"
            }
        } else if id_is(e, "minecraft:spectral_arrow") {
            "minecraft:spectral_arrow"
        } else {
            // The trident's value is cast to the new type: its typed Trident field
            // is written as `item`, and the raw Trident comes back from the remainder.
            if id_is(e, "minecraft:trident") {
                rename(e, "Trident", "item");
                super::drop_typed(e, "Trident");
            }
            return;
        };
        let stack = Tag::compound([("id", string(item)), ("Count", Tag::Int(1))]);
        set(e, "item", stack);
        // The new stack is read with the output schema (V3685): its hook.
        if let Some(s) = e.get_mut("item") {
            items::add_names(s);
        }
        reread(k, e);
    });
}

// ------------------------------------------------------------------ 3692 renames

/// `LegacyBlockRenameFix`: block names, block states and flat block states.
pub(super) fn block_rename(cx: &mut Cx, table: &[(&str, &'static str)]) {
    let r = renamer(table);
    cx.rename(Ty::BlockName, &r);
    cx.each(Ty::BlockState, |v| {
        if let Some(name) = as_str(v.get("Name"))
            && let Some(new) = r(&name)
        {
            set(v, "Name", string(new));
        }
    });
    cx.each(Ty::FlatBlockState, |v| {
        if let Some(s) = as_str(Some(v)) {
            let props = s.find('[').filter(|&i| i > 0);
            let nbt = s.find('{').filter(|&i| i > 0);
            let end = match (props, nbt) {
                (Some(a), Some(b)) => a.min(b),
                (Some(a), None) => a,
                (None, Some(b)) => b,
                (None, None) => s.len(),
            };
            let name = &s[..end];
            let new = r(name).map_or_else(|| name.to_owned(), str::to_owned);
            *v = string(&format!("{new}{}", &s[end..]));
        }
    });
}

// ------------------------------------------------------------------ 3803

fn rename_sweeping(cx: &mut Cx) {
    cx.each(Ty::ItemStack, |s| {
        let Some(tag) = s.get_mut("tag") else {
            return;
        };
        for field in ["Enchantments", "StoredEnchantments"] {
            if let Some(list) = tag.get(field)
                && is_list(Some(list))
            {
                let fixed = stream(Some(list))
                    .into_iter()
                    .map(|mut e| {
                        if let Some(id) = as_str(e.get("id")) {
                            let new = if namespaced(&id) == "minecraft:sweeping" {
                                "minecraft:sweeping_edge".into()
                            } else {
                                id
                            };
                            set(&mut e, "id", string(&new));
                        }
                        e
                    })
                    .collect();
                set(tag, field, Tag::List(fixed));
            }
        }
    });
}

// ------------------------------------------------------------------ 3807.1

/// `ExtraDataFixUtils.fixBlockPos`.
pub(super) fn fix_block_pos(v: Tag) -> Tag {
    let (x, y, z) = (
        v.get("X").and_then(as_number),
        v.get("Y").and_then(as_number),
        v.get("Z").and_then(as_number),
    );
    match (x, y, z) {
        (Some(x), Some(y), Some(z)) => int_list(&[x.int_value(), y.int_value(), z.int_value()]),
        _ => v,
    }
}
/// `ExtraDataFixUtils.fixInlineBlockPos`.
pub(super) fn fix_inline_block_pos(v: &mut Tag, fx: &str, fy: &str, fz: &str, new: &str) {
    let (x, y, z) = (
        v.get(fx).and_then(as_number),
        v.get(fy).and_then(as_number),
        v.get(fz).and_then(as_number),
    );
    if let (Some(x), Some(y), Some(z)) = (x, y, z) {
        remove(v, fx);
        remove(v, fy);
        remove(v, fz);
        set(
            v,
            new,
            int_list(&[x.int_value(), y.int_value(), z.int_value()]),
        );
    }
}

fn map_banner_block_pos(cx: &mut Cx) {
    cx.each(Ty::SavedMapData, |v| {
        if let Some(d) = v.get_mut("data")
            && let Some(Tag::List(banners)) = d.get_mut("banners")
        {
            for b in banners {
                super::dynamic::update(b, "Pos", fix_block_pos);
            }
        }
    });
}

// ------------------------------------------------------------------ 3808, 3809, 3812

fn horse_body_armor(cx: &mut Cx, id: &str, field: &str, clear_armor_items: bool) {
    let k = cx.k;
    cx.entity(id, |e| {
        horse_body_armor_entity(e, field, clear_armor_items);
        reread(k, e);
    });
}

fn horse_body_armor_entity(e: &mut Tag, field: &str, clear_armor_items: bool) {
    {
        let Some(armor) = e.get(field).cloned() else {
            return;
        };
        remove(e, field);
        if clear_armor_items {
            super::dynamic::update(e, "ArmorItems", |list| {
                Tag::List(
                    stream(Some(&list))
                        .into_iter()
                        .enumerate()
                        .map(|(i, x)| if i == 2 { empty_map() } else { x })
                        .collect(),
                )
            });
            super::dynamic::update(e, "ArmorDropChances", |list| {
                Tag::List(
                    stream(Some(&list))
                        .into_iter()
                        .enumerate()
                        .map(|(i, x)| {
                            if i == 2 {
                                Tag::Float(0.085f32.to_bits())
                            } else {
                                x
                            }
                        })
                        .collect(),
                )
            });
        }
        set(e, "body_armor_item", armor);
        set(e, "body_armor_drop_chance", Tag::Float(2.0f32.to_bits()));
    }
}

fn chested_horse_slots(cx: &mut Cx) {
    cx.each(Ty::Entity, |e| {
        if ![
            "minecraft:llama",
            "minecraft:trader_llama",
            "minecraft:mule",
            "minecraft:donkey",
        ]
        .iter()
        .any(|id| id_is(e, id))
        {
            return;
        }
        if let Some(Tag::List(items)) = e.get_mut("Items") {
            for item in items {
                if let Some(slot) = item.get("Slot") {
                    let n = as_number(slot).map_or(2, |n| n.int_value());
                    set(item, "Slot", Tag::Byte((n - 2) as i8));
                }
            }
        }
    });
}

fn wolf_health(cx: &mut Cx) {
    cx.entity("minecraft:wolf", |e| {
        let mut adjusted = false;
        super::dynamic::update(e, "Attributes", |list| {
            Tag::List(
                stream(Some(&list))
                    .into_iter()
                    .map(|mut a| {
                        if namespaced(&as_string(a.get("Name"), ""))
                            == "minecraft:generic.max_health"
                        {
                            super::dynamic::update(&mut a, "Base", |base| {
                                if as_double(Some(&base), 0.0) == 20.0 {
                                    adjusted = true;
                                    Tag::Double(40.0f64.to_bits())
                                } else {
                                    base
                                }
                            });
                        }
                        a
                    })
                    .collect(),
            )
        });
        if adjusted {
            super::dynamic::update(e, "Health", |h| {
                Tag::Float((as_float(Some(&h), 0.0) * 2.0).to_bits())
            });
        }
    });
}

// ------------------------------------------------------------------ 3813

const PATROLLING_MOBS: [&str; 6] = [
    "minecraft:witch",
    "minecraft:ravager",
    "minecraft:pillager",
    "minecraft:illusioner",
    "minecraft:evoker",
    "minecraft:vindicator",
];

fn rename_block_pos(v: &mut Tag, fields: &[(&str, &str)]) {
    for (old, new) in fields {
        rename_and_fix(v, old, new, |p| Some(fix_block_pos(p)));
    }
}

fn block_pos_format(cx: &mut Cx) {
    cx.each(Ty::Entity, |e| {
        if id_is(e, "minecraft:bee") {
            rename_block_pos(e, &[("HivePos", "hive_pos"), ("FlowerPos", "flower_pos")]);
        }
        if id_is(e, "minecraft:end_crystal") {
            rename_block_pos(e, &[("BeamTarget", "beam_target")]);
        }
        if id_is(e, "minecraft:wandering_trader") {
            rename_block_pos(e, &[("WanderTarget", "wander_target")]);
        }
        if PATROLLING_MOBS.iter().any(|id| id_is(e, id)) {
            rename_block_pos(e, &[("PatrolTarget", "patrol_target")]);
        }
        rename_block_pos(e, &[("Leash", "leash")]);
    });
    cx.block_entity("minecraft:beehive", |be| {
        rename_block_pos(be, &[("FlowerPos", "flower_pos")])
    });
    cx.block_entity("minecraft:end_gateway", |be| {
        rename_block_pos(be, &[("ExitPortal", "exit_portal")])
    });
    cx.each(Ty::SavedMapData, |v| {
        if let Some(d) = v.get_mut("data") {
            super::dynamic::update(d, "frames", |frames| {
                Tag::List(
                    stream(Some(&frames))
                        .into_iter()
                        .map(|mut f| {
                            rename_and_fix(&mut f, "Pos", "pos", |p| Some(fix_block_pos(p)));
                            rename(&mut f, "Rotation", "rotation");
                            rename(&mut f, "EntityId", "entity_id");
                            f
                        })
                        .collect(),
                )
            });
            super::dynamic::update(d, "banners", |banners| {
                Tag::List(
                    stream(Some(&banners))
                        .into_iter()
                        .map(|mut b| {
                            rename(&mut b, "Pos", "pos");
                            rename(&mut b, "Color", "color");
                            rename(&mut b, "Name", "name");
                            b
                        })
                        .collect(),
                )
            });
        }
    });
    cx.item(
        |id| id == "minecraft:compass",
        |s| {
            if let Some(tag) = s.get_mut("tag") {
                super::dynamic::update(tag, "LodestonePos", fix_block_pos);
            }
        },
    );
}

// ------------------------------------------------------------------ 3814

pub(super) fn attributes_rename_legacy(
    cx: &mut Cx,
    rename_fn: &dyn Fn(&str) -> Option<&'static str>,
) {
    let fix_name = |v: &mut Tag, field: &str| {
        if let Some(name) = as_str(v.get(field))
            && let Some(new) = rename_fn(&name)
        {
            set(v, field, string(new));
        }
    };
    cx.each(Ty::ItemStack, |s| {
        if let Some(tag) = s.get_mut("tag")
            && let Some(Tag::List(mods)) = tag.get_mut("AttributeModifiers")
        {
            for m in mods {
                fix_name(m, "AttributeName");
            }
        }
    });
    let fix_entity = |e: &mut Tag| {
        if let Some(Tag::List(attrs)) = e.get_mut("Attributes") {
            for a in attrs {
                fix_name(a, "Name");
            }
        }
    };
    cx.each(Ty::Entity, fix_entity);
    cx.each(Ty::Player, fix_entity);
}

// ------------------------------------------------------------------ 3818

fn beehive_fields(cx: &mut Cx) {
    cx.block_entity("minecraft:beehive", |be| {
        // The typed Bees list moves to `bees` (the schema renames it); the
        // remainder loses Bees, each bee loses EntityData's old name.
        if let Some(Tag::List(bees)) = remove(be, "Bees") {
            let bees = bees
                .into_iter()
                .map(|mut bee| {
                    let data = remove(&mut bee, "EntityData");
                    rename(&mut bee, "TicksInHive", "ticks_in_hive");
                    rename(&mut bee, "MinOccupationTicks", "min_ticks_in_hive");
                    if let Some(data) = data {
                        set(&mut bee, "entity_data", data);
                    }
                    bee
                })
                .collect();
            set(be, "bees", Tag::List(bees));
        }
    });
}

/// `EmptyItemInHotbarFix`: a saved toolbar's air or zero-count stacks become
/// the empty compound.
fn empty_item_in_hotbar(cx: &mut Cx) {
    if !matches!(cx.root, Ty::Hotbar) {
        return;
    }
    cx.each(Ty::ItemStack, |stack| {
        let air = as_str(stack.get("id")).is_none_or(|id| id == "minecraft:air");
        let empty = as_int(stack.get("Count"), 0) <= 0;
        if air || empty {
            *stack = empty_map();
        }
    });
}

const PATTERN_IDS: [(&str, &str); 41] = [
    ("b", "minecraft:base"),
    ("bl", "minecraft:square_bottom_left"),
    ("br", "minecraft:square_bottom_right"),
    ("tl", "minecraft:square_top_left"),
    ("tr", "minecraft:square_top_right"),
    ("bs", "minecraft:stripe_bottom"),
    ("ts", "minecraft:stripe_top"),
    ("ls", "minecraft:stripe_left"),
    ("rs", "minecraft:stripe_right"),
    ("cs", "minecraft:stripe_center"),
    ("ms", "minecraft:stripe_middle"),
    ("drs", "minecraft:stripe_downright"),
    ("dls", "minecraft:stripe_downleft"),
    ("ss", "minecraft:small_stripes"),
    ("cr", "minecraft:cross"),
    ("sc", "minecraft:straight_cross"),
    ("bt", "minecraft:triangle_bottom"),
    ("tt", "minecraft:triangle_top"),
    ("bts", "minecraft:triangles_bottom"),
    ("tts", "minecraft:triangles_top"),
    ("ld", "minecraft:diagonal_left"),
    ("rd", "minecraft:diagonal_up_right"),
    ("lud", "minecraft:diagonal_up_left"),
    ("rud", "minecraft:diagonal_right"),
    ("mc", "minecraft:circle"),
    ("mr", "minecraft:rhombus"),
    ("vh", "minecraft:half_vertical"),
    ("hh", "minecraft:half_horizontal"),
    ("vhr", "minecraft:half_vertical_right"),
    ("hhb", "minecraft:half_horizontal_bottom"),
    ("bo", "minecraft:border"),
    ("cbo", "minecraft:curly_border"),
    ("gra", "minecraft:gradient"),
    ("gru", "minecraft:gradient_up"),
    ("bri", "minecraft:bricks"),
    ("glb", "minecraft:globe"),
    ("cre", "minecraft:creeper"),
    ("sku", "minecraft:skull"),
    ("flo", "minecraft:flower"),
    ("moj", "minecraft:mojang"),
    ("pig", "minecraft:piglin"),
];

/// `ExtraDataFixUtils.dyeColorIdToName`.
pub(super) fn dye_color(id: i32) -> &'static str {
    match id {
        1 => "orange",
        2 => "magenta",
        3 => "light_blue",
        4 => "yellow",
        5 => "lime",
        6 => "pink",
        7 => "gray",
        8 => "light_gray",
        9 => "cyan",
        10 => "purple",
        11 => "blue",
        12 => "brown",
        13 => "green",
        14 => "red",
        15 => "black",
        _ => "white",
    }
}

pub(super) fn banner_layer(mut v: Tag) -> Tag {
    rename_and_fix(&mut v, "Pattern", "pattern", |p| {
        Some(match as_str(Some(&p)) {
            Some(id) => string(
                PATTERN_IDS
                    .iter()
                    .find(|(k, _)| *k == id)
                    .map_or(id.as_str(), |(_, n)| n),
            ),
            None => p,
        })
    });
    let color = dye_color(as_int(v.get("Color"), 0));
    set(&mut v, "color", string(color));
    remove(&mut v, "Color");
    v
}

fn banner_patterns(cx: &mut Cx) {
    cx.block_entity("minecraft:banner", |be| {
        rename_and_fix(be, "Patterns", "patterns", |p| {
            Some(Tag::List(
                stream(Some(&p)).into_iter().map(banner_layer).collect(),
            ))
        });
    });
}

fn tipped_arrow_potion(cx: &mut Cx) {
    let k = cx.k;
    cx.entity("minecraft:arrow", |e| {
        tipped_arrow_potion_entity(e);
        reread(k, e);
    });
}

fn tipped_arrow_potion_entity(e: &mut Tag) {
    {
        let potion = e.get("Potion").cloned();
        let effects = e.get("custom_potion_effects").cloned();
        let color = e.get("Color").cloned();
        if potion.is_none() && effects.is_none() && color.is_none() {
            return;
        }
        remove(e, "Potion");
        remove(e, "custom_potion_effects");
        remove(e, "Color");
        super::dynamic::update(e, "item", |mut stack| {
            let mut tag = super::dynamic::or_empty_map(stack.get("tag"));
            if let Some(p) = potion {
                set(&mut tag, "Potion", p);
            }
            if let Some(c) = effects {
                set(&mut tag, "custom_potion_effects", c);
            }
            if let Some(c) = color {
                set(&mut tag, "CustomPotionColor", c);
            }
            set(&mut stack, "tag", tag);
            stack
        });
    }
}

fn particle_unflattening(cx: &mut Cx) {
    cx.each(Ty::Particle, |p| {
        if let Some(s) = as_str(Some(p)) {
            *p = items::unflatten_particle(&s);
        }
    });
}

fn area_effect_cloud_potion(cx: &mut Cx) {
    cx.entity("minecraft:area_effect_cloud", |e| {
        let color = remove(e, "Color");
        let effects = remove(e, "effects");
        let potion = remove(e, "Potion");
        if color.is_none() && effects.is_none() && potion.is_none() {
            return;
        }
        let mut contents = empty_map();
        if let Some(c) = color {
            set(&mut contents, "custom_color", c);
        }
        if let Some(c) = effects {
            set(&mut contents, "custom_effects", c);
        }
        if let Some(p) = potion {
            set(&mut contents, "potion", p);
        }
        set(e, "potion_contents", contents);
    });
}

// ------------------------------------------------------------------ 3820

fn player_head_profile(cx: &mut Cx) {
    cx.block_entity("minecraft:skull", |be| {
        let profile = be
            .get("SkullOwner")
            .or_else(|| be.get("ExtraType"))
            .cloned();
        let Some(profile) = profile else {
            return;
        };
        remove(be, "SkullOwner");
        remove(be, "ExtraType");
        set(be, "profile", items::fix_profile(&profile));
    });
}

/// `DataComponentRemainderFix`: one component of every component map.
pub(super) fn component_fix(cx: &mut Cx, id: &str, new_id: &str, f: impl Fn(Tag) -> Option<Tag>) {
    cx.each(Ty::DataComponents, |c| {
        if let Some(component) = remove(c, id)
            && let Some(new) = f(component)
        {
            set(c, new_id, new);
        }
    });
}

fn lodestone_compass_component(cx: &mut Cx) {
    component_fix(
        cx,
        "minecraft:lodestone_target",
        "minecraft:lodestone_tracker",
        |mut v| {
            let pos = remove(&mut v, "pos");
            let dimension = remove(&mut v, "dimension");
            if let (Some(pos), Some(dimension)) = (pos, dimension) {
                set(
                    &mut v,
                    "target",
                    Tag::compound([("pos", pos), ("dimension", dimension)]),
                );
            }
            Some(v)
        },
    );
}

// ------------------------------------------------------------------ 3825

const MAP_NAMES: [&str; 11] = [
    "filled_map.buried_treasure",
    "filled_map.explorer_jungle",
    "filled_map.explorer_swamp",
    "filled_map.mansion",
    "filled_map.monument",
    "filled_map.trial_chambers",
    "filled_map.village_desert",
    "filled_map.village_plains",
    "filled_map.village_savanna",
    "filled_map.village_snowy",
    "filled_map.village_taiga",
];

/// `LegacyComponentDataFixUtils.extractTranslationString`.
pub(super) fn translation_key(component: &Tag) -> Option<String> {
    let Tag::String(s) = component else {
        return None;
    };
    match json::parse_lenient(&s.0)? {
        j @ json::Json::Obj(_) => match j.get("translate")? {
            json::Json::Str(k) => Some(String::from_utf16_lossy(k)),
            json::Json::Num(n) => Some(n.clone()),
            json::Json::Bool(b) => Some(b.to_string()),
            _ => None,
        },
        _ => None,
    }
}

fn item_custom_name_to_item_name(cx: &mut Cx) {
    cx.each(Ty::ItemStack, |s| {
        let id = namespaced(&as_str(s.get("id")).unwrap_or_default());
        let expected: &dyn Fn(&str) -> bool = match id.as_str() {
            "minecraft:white_banner" => &|k: &str| k == "block.minecraft.ominous_banner",
            "minecraft:filled_map" => &|k: &str| MAP_NAMES.contains(&k),
            _ => return,
        };
        if let Some(c) = s.get_mut("components")
            && c.get("minecraft:custom_name")
                .and_then(translation_key)
                .is_some_and(|k| expected(&k))
        {
            rename(c, "minecraft:custom_name", "minecraft:item_name");
        }
    });
}

fn banner_custom_name_to_item_name(cx: &mut Cx) {
    cx.block_entity("minecraft:banner", |be| {
        let Some(name) = be.get("CustomName").cloned() else {
            return;
        };
        if translation_key(&name).as_deref() != Some("block.minecraft.ominous_banner") {
            return;
        }
        set(
            be,
            "components",
            Tag::compound([
                ("minecraft:item_name", name),
                ("minecraft:hide_additional_tooltip", empty_map()),
            ]),
        );
        remove(be, "CustomName");
    });
}

fn trial_spawner_config(cx: &mut Cx) {
    cx.block_entity("minecraft:trial_spawner", |be| {
        let mut config = empty_map();
        let mut moved = false;
        for k in [
            "spawn_range",
            "total_mobs",
            "simultaneous_mobs",
            "total_mobs_added_per_player",
            "simultaneous_mobs_added_per_player",
            "ticks_between_spawn",
            "spawn_potentials",
            "loot_tables_to_eject",
            "items_to_drop_when_ominous",
        ] {
            if let Some(v) = remove(be, k) {
                set(&mut config, k, v);
                moved = true;
            }
        }
        if moved {
            set(be, "normal_config", config);
        }
    });
}

// ------------------------------------------------------------------ 3828, 3833

fn empty_item_in_villager_trade(cx: &mut Cx) {
    cx.each(Ty::VillagerTrade, |t| {
        let buy_b = super::dynamic::or_empty_map(t.get("buyB"));
        let id = namespaced(&as_string(buy_b.get("id"), "minecraft:air"));
        let count = as_int(buy_b.get("count"), 0);
        if id == "minecraft:air" || count == 0 {
            remove(t, "buyB");
        }
    });
}

fn remove_empty_brushable_item(cx: &mut Cx) {
    cx.block_entity("minecraft:brushable_block", |be| {
        if let Some(item) = be.get("item") {
            let id = namespaced(&as_string(item.get("id"), "minecraft:air"));
            if id == "minecraft:air" || as_int(item.get("count"), 0) == 0 {
                remove(be, "item");
            }
        }
    });
}

// ------------------------------------------------------------------ scoreboard objective hooks

/// `V1451_6.UNPACK_OBJECTIVE_ID`.
pub(super) fn unpack_objective(v: &mut Tag) {
    let Some(name) = as_str(v.get("CriteriaName")) else {
        return;
    };
    let (ty, id) = match name.find(':') {
        None => ("_special".to_owned(), name.clone()),
        Some(colon) => match (
            by_separator(&name[..colon]),
            by_separator(&name[colon + 1..]),
        ) {
            (Some(t), Some(i)) => (t, i),
            _ => ("_special".to_owned(), name.clone()),
        },
    };
    set(
        v,
        "CriteriaType",
        Tag::compound([("type", string(&ty)), ("id", string(&id))]),
    );
}

/// `Identifier.bySeparator(s, '.')`: namespace before the first dot.
fn by_separator(s: &str) -> Option<String> {
    let (ns, path) = match s.find('.') {
        Some(i) => (&s[..i], &s[i + 1..]),
        None => ("minecraft", s),
    };
    let ns = if ns.is_empty() { "minecraft" } else { ns };
    super::identifier(&format!("{ns}:{path}")).map(|(n, p)| format!("{n}:{p}"))
}

/// `V1451_6.REPACK_OBJECTIVE_ID`.
pub(super) fn repack_objective(v: &mut Tag) {
    let Some(ct) = v.get("CriteriaType") else {
        return;
    };
    let (Some(ty), Some(id)) = (as_str(ct.get("type")), as_str(ct.get("id"))) else {
        return;
    };
    let name = if ty == "_special" {
        id
    } else {
        format!("{}:{}", pack_with_dot(&ty), pack_with_dot(&id))
    };
    set(v, "CriteriaName", string(&name));
    remove(v, "CriteriaType");
}
fn pack_with_dot(location: &str) -> String {
    match super::identifier(location) {
        Some((ns, path)) => format!("{ns}.{path}"),
        None => location.to_owned(),
    }
}
