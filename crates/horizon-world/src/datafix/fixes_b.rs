//! Fixes of 1.21 to 1.21.5 (keys 3938 to 4314), text components apart.
use super::dynamic::{
    as_bool, as_double, as_float, as_int, as_long, as_number, as_str, as_string, empty_map,
    is_empty_map, is_list, namespaced, or_empty_map, remove, rename, rename_and_fix, set, stream,
    string, update,
};
use super::fixes_a::{component_fix, dye_color, fix_inline_block_pos, translation_key};
use super::items::java_hash_order;
use super::schema::{Key, Ty, key};
use super::{Cx, FixFn, id_is, reread};
use crate::nbt::{Compound, Tag};

pub(super) fn register(out: &mut Vec<(Key, FixFn)>) {
    // ProjectileStoredWeaponFix only writes and reads arrows with the new schema.
    out.push((key(3938, 0), |cx| {
        let k = cx.k;
        cx.each(Ty::Entity, |e| {
            if id_is(e, "minecraft:arrow") || id_is(e, "minecraft:spectral_arrow") {
                reread(k, e);
            }
        });
    }));
    out.push((key(3939, 0), |cx| {
        feature_flag_remove(cx, &["minecraft:update_1_21"])
    }));
    out.push((key(3945, 0), attribute_modifier_ids));
    out.push((key(3945, 0), jukebox_ticks));
    out.push((key(4054, 0), ominous_banner_rarity));
    out.push((key(4055, 0), |cx| {
        attributes_rename(cx, true, &attribute_id_prefix)
    }));
    out.push((key(4059, 0), food_to_consumable));
    out.push((key(4061, 0), trial_spawner_config_in_registry));
    out.push((key(4064, 0), |cx| {
        component_fix(
            cx,
            "minecraft:fire_resistant",
            "minecraft:damage_resistant",
            |_| Some(Tag::compound([("types", string("#minecraft:is_fire"))])),
        );
    }));
    out.push((key(4067, 0), boat_split));
    out.push((key(4067, 0), |cx| {
        feature_flag_remove(cx, &["minecraft:bundle"])
    }));
    out.push((key(4068, 0), |cx| {
        component_fix(cx, "minecraft:lock", "minecraft:lock", fix_lock)
    }));
    out.push((key(4068, 0), |cx| {
        cx.each(Ty::BlockEntity, |be| {
            rename_and_fix(be, "Lock", "lock", fix_lock)
        });
    }));
    out.push((key(4081, 0), |cx| {
        cx.entity("minecraft:salmon", |e| {
            if as_string(e.get("type"), "medium") != "large" {
                set(e, "type", string("medium"));
            }
        });
    }));
    out.push((key(4173, 0), |cx| {
        cx.entity("minecraft:tnt_minecart", |e| rename(e, "TNTFuse", "fuse"))
    }));
    out.push((key(4175, 0), |cx| {
        cx.each(Ty::DataComponents, |c| {
            if let Some(eq) = c.get_mut("minecraft:equippable") {
                rename(eq, "model", "asset_id");
            }
        });
    }));
    out.push((key(4175, 0), |cx| {
        cx.each(Ty::DataComponents, |c| {
            update(c, "minecraft:custom_model_data", |cmd| {
                let value = as_number(&cmd).map_or(0.0, |n| n.float_value());
                Tag::compound([("floats", Tag::List(vec![Tag::Float(value.to_bits())]))])
            });
        });
    }));
    out.push((key(4176, 0), |cx| {
        cx.each(Ty::BlockEntity, |be| {
            if let Some(lock) = remove(be, "lock")
                && !broken_lock(&lock)
            {
                set(be, "lock", lock);
            }
        });
    }));
    out.push((key(4176, 0), |cx| {
        component_fix(cx, "minecraft:lock", "minecraft:lock", |l| {
            (!broken_lock(&l)).then_some(l)
        });
    }));
    out.push((key(4180, 0), |cx| {
        feature_flag_remove(cx, &["minecraft:winter_drop"])
    }));
    for id in [
        "minecraft:furnace",
        "minecraft:smoker",
        "minecraft:blast_furnace",
    ] {
        let _ = id;
    }
    out.push((key(4181, 0), |cx| {
        furnace_burn_time(cx, "minecraft:furnace")
    }));
    out.push((key(4181, 0), |cx| furnace_burn_time(cx, "minecraft:smoker")));
    out.push((key(4181, 0), |cx| {
        furnace_burn_time(cx, "minecraft:blast_furnace")
    }));
    out.push((key(4187, 0), |cx| {
        base_value(
            cx,
            "minecraft:villager",
            "minecraft:follow_range",
            48.0,
            16.0,
        )
    }));
    out.push((key(4187, 0), |cx| {
        base_value(cx, "minecraft:bee", "minecraft:follow_range", 48.0, 16.0)
    }));
    out.push((key(4187, 0), |cx| {
        base_value(cx, "minecraft:allay", "minecraft:follow_range", 48.0, 16.0)
    }));
    out.push((key(4187, 0), |cx| {
        base_value(cx, "minecraft:llama", "minecraft:follow_range", 40.0, 16.0)
    }));
    out.push((key(4187, 0), |cx| {
        base_value(
            cx,
            "minecraft:piglin_brute",
            "minecraft:follow_range",
            16.0,
            12.0,
        )
    }));
    out.push((key(4187, 0), |cx| {
        base_value(cx, "minecraft:warden", "minecraft:follow_range", 16.0, 24.0)
    }));
    super::text::register(out);
    out.push((key(4293, 0), drop_chances_format));
    out.push((key(4294, 0), |cx| {
        block_property_rename(
            cx,
            "minecraft:creaking_heart",
            "active",
            "creaking_heart_state",
            &|v| {
                if v == "true" {
                    "awake".into()
                } else {
                    "uprooted".into()
                }
            },
        );
    }));
    out.push((key(4296, 0), |cx| {
        cx.entity("minecraft:area_effect_cloud", |e| {
            set(e, "potion_duration_scale", Tag::Float(0.25f32.to_bits()))
        });
    }));
    out.push((key(4297, 0), forced_chunk_to_ticket));
    out.push((key(4299, 0), entity_spawner_item_variants));
    out.push((key(4300, 0), saddle_equipment_slot));
    out.push((key(4301, 0), equipment_format));
    out.push((key(4303, 0), |cx| {
        cx.each_out(Ty::Entity, fall_distance_to_double)
    }));
    out.push((key(4303, 0), |cx| {
        cx.each_out(Ty::Player, fall_distance_to_double)
    }));
    out.push((key(4305, 0), |cx| {
        block_property_rename(
            cx,
            "minecraft:test_block",
            "test_block_mode",
            "mode",
            &|v| v.to_owned(),
        );
    }));
    out.push((key(4306, 0), thrown_potion_split));
    out.push((key(4307, 0), tooltip_display));
    out.push((key(4309, 0), raid_renames));
    out.push((key(4309, 0), chunk_ticket_unpack_pos));
    out.push((key(4311, 0), |cx| {
        advancements_rename(cx, &|id| {
            (namespaced(id) == "minecraft:nether/use_lodestone")
                .then(|| "minecraft:adventure/use_lodestone".into())
        });
    }));
    out.push((key(4312, 0), player_equipment));
    out.push((key(4314, 0), inline_block_pos_format));
}

// ------------------------------------------------------------------ 3939, 4067, 4180, 4424

pub(super) fn feature_flag_remove(cx: &mut Cx, flags: &[&str]) {
    cx.each(Ty::LightweightLevel, |level| {
        let mut inactive: Vec<Tag> = stream(level.get("removed_features"));
        if let Some(features) = level.get("enabled_features")
            && is_list(Some(features))
        {
            let kept: Vec<Tag> = stream(Some(features))
                .into_iter()
                .filter(|f| match as_str(Some(f)) {
                    Some(s) if flags.contains(&s.as_str()) => {
                        inactive.push(string(&s));
                        false
                    }
                    _ => true,
                })
                .collect();
            set(level, "enabled_features", Tag::List(kept));
        }
        if !inactive.is_empty() {
            set(level, "removed_features", Tag::List(inactive));
        }
    });
}

// ------------------------------------------------------------------ 3945

fn uuid_text(u: [i32; 4]) -> String {
    let most = (i64::from(u[0]) << 32) | (i64::from(u[1]) & 0xFFFF_FFFF);
    let least = (i64::from(u[2]) << 32) | (i64::from(u[3]) & 0xFFFF_FFFF);
    let (m, l) = (most as u64, least as u64);
    format!(
        "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
        m >> 32,
        (m >> 16) & 0xFFFF,
        m & 0xFFFF,
        l >> 48,
        l & 0xFFFF_FFFF_FFFF
    )
}

const MODIFIER_IDS: &[(&str, &str)] = &[
    (
        "736565d2-e1a7-403d-a3f8-1aeb3e302542",
        "minecraft:creative_mode_block_range",
    ),
    (
        "98491ef6-97b1-4584-ae82-71a8cc85cf73",
        "minecraft:creative_mode_entity_range",
    ),
    (
        "91aeaa56-376b-4498-935b-2f7f68070635",
        "minecraft:effect.speed",
    ),
    (
        "7107de5e-7ce8-4030-940e-514c1f160890",
        "minecraft:effect.slowness",
    ),
    (
        "af8b6e3f-3328-4c0a-aa36-5ba2bb9dbef3",
        "minecraft:effect.haste",
    ),
    (
        "55fced67-e92a-486e-9800-b47f202c4386",
        "minecraft:effect.mining_fatigue",
    ),
    (
        "648d7064-6a60-4f59-8abe-c2c23a6dd7a9",
        "minecraft:effect.strength",
    ),
    (
        "c0105bf3-aef8-46b0-9ebc-92943757ccbe",
        "minecraft:effect.jump_boost",
    ),
    (
        "22653b89-116e-49dc-9b6b-9971489b5be5",
        "minecraft:effect.weakness",
    ),
    (
        "5d6f0ba2-1186-46ac-b896-c61c5cee99cc",
        "minecraft:effect.health_boost",
    ),
    (
        "eae29cf0-701e-4ed6-883a-96f798f3dab5",
        "minecraft:effect.absorption",
    ),
    (
        "03c3c89d-7037-4b42-869f-b146bcb64d2e",
        "minecraft:effect.luck",
    ),
    (
        "cc5af142-2bd2-4215-b636-2605aed11727",
        "minecraft:effect.unluck",
    ),
    ("6555be74-63b3-41f1-a245-77833b3c2562", "minecraft:evil"),
    (
        "1eaf83ff-7207-4596-b37a-d7a07b3ec4ce",
        "minecraft:powder_snow",
    ),
    (
        "662a6b8d-da3e-4c1c-8813-96ea6097278d",
        "minecraft:sprinting",
    ),
    (
        "020e0dfb-87ae-4653-9556-831010e291a0",
        "minecraft:attacking",
    ),
    ("766bfa64-11f3-11ea-8d71-362b9e155667", "minecraft:baby"),
    ("7e0292f2-9434-48d5-a29f-9583af7df27f", "minecraft:covered"),
    (
        "9e362924-01de-4ddd-a2b2-d0f7a405a174",
        "minecraft:suffocating",
    ),
    ("5cd17e52-a79a-43d3-a529-90fde04b181e", "minecraft:drinking"),
    ("b9766b59-9566-4402-bc1f-2ee2a276d836", "minecraft:baby"),
    (
        "49455a49-7ec5-45ba-b886-3b90b23a1718",
        "minecraft:attacking",
    ),
    (
        "845db27c-c624-495f-8c9f-6020a9a58b6b",
        "minecraft:armor.boots",
    ),
    (
        "d8499b04-0e66-4726-ab29-64469d734e0d",
        "minecraft:armor.leggings",
    ),
    (
        "9f3d476d-c118-4544-8365-64846904b48e",
        "minecraft:armor.chestplate",
    ),
    (
        "2ad3f246-fee1-4e67-b886-69fd380bb150",
        "minecraft:armor.helmet",
    ),
    (
        "c1c72771-8b8e-ba4a-ace0-81a93c8928b2",
        "minecraft:armor.body",
    ),
    (
        "b572ecd2-ac0c-4071-abde-9594af072a37",
        "minecraft:enchantment.fire_protection",
    ),
    (
        "40a9968f-5c66-4e2f-b7f4-2ec2f4b3e450",
        "minecraft:enchantment.blast_protection",
    ),
    (
        "07a65791-f64d-4e79-86c7-f83932f007ec",
        "minecraft:enchantment.respiration",
    ),
    (
        "60b1b7db-fffd-4ad0-817c-d6c6a93d8a45",
        "minecraft:enchantment.aqua_affinity",
    ),
    (
        "11dc269a-4476-46c0-aff3-9e17d7eb6801",
        "minecraft:enchantment.depth_strider",
    ),
    (
        "87f46a96-686f-4796-b035-22e16ee9e038",
        "minecraft:enchantment.soul_speed",
    ),
    (
        "b9716dbd-50df-4080-850e-70347d24e687",
        "minecraft:enchantment.soul_speed",
    ),
    (
        "92437d00-c3a7-4f2e-8f6c-1f21585d5dd0",
        "minecraft:enchantment.swift_sneak",
    ),
    (
        "5d3d087b-debe-4037-b53e-d84f3ff51f17",
        "minecraft:enchantment.sweeping_edge",
    ),
    (
        "3ceb37c0-db62-46b5-bd02-785457b01d96",
        "minecraft:enchantment.efficiency",
    ),
    (
        "cb3f55d3-645c-4f38-a497-9c13a33db5cf",
        "minecraft:base_attack_damage",
    ),
    (
        "fa233e1c-4180-4865-b01b-bcce9785aca3",
        "minecraft:base_attack_speed",
    ),
];
const MODIFIER_NAMES: &[(&str, &str)] = &[
    ("Random spawn bonus", "minecraft:random_spawn_bonus"),
    (
        "Random zombie-spawn bonus",
        "minecraft:zombie_random_spawn_bonus",
    ),
    ("Leader zombie bonus", "minecraft:leader_zombie_bonus"),
    (
        "Zombie reinforcement callee charge",
        "minecraft:reinforcement_callee_charge",
    ),
    (
        "Zombie reinforcement caller charge",
        "minecraft:reinforcement_caller_charge",
    ),
];

/// `asIntStream` of a UUID field, as `uuidFromIntArray` wants it.
fn uuid_of(v: Option<&Tag>) -> Option<[i32; 4]> {
    let ints: Vec<i32> = match v {
        Some(Tag::IntArray(a)) => a.clone(),
        other => stream(other)
            .iter()
            .map(|t| as_number(t).map_or(0, |n| n.int_value()))
            .collect(),
    };
    (ints.len() == 4).then(|| [ints[0], ints[1], ints[2], ints[3]])
}

/// `AttributeModifierIdFix.fixModifiers`: an insertion-ordered map by id.
fn fix_modifiers(modifiers: Vec<Tag>) -> Vec<Tag> {
    let mut converted: Vec<(String, Tag)> = Vec::new();
    let put = |c: &mut Vec<(String, Tag)>, id: String, v: Tag| {
        if let Some(e) = c.iter_mut().find(|(k, _)| *k == id) {
            e.1 = v;
        } else {
            c.push((id, v));
        }
    };
    for mut m in modifiers {
        let uuid = uuid_of(m.get("uuid")).map(uuid_text);
        let name = as_string(m.get("name"), "");
        let from_uuid = uuid
            .as_deref()
            .and_then(|u| MODIFIER_IDS.iter().find(|(k, _)| *k == u).map(|(_, v)| *v));
        let from_name = MODIFIER_NAMES
            .iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| *v);
        if let Some(id) = from_uuid {
            set(&mut m, "id", string(id));
            remove(&mut m, "uuid");
            remove(&mut m, "name");
            put(&mut converted, id.to_owned(), m);
        } else if let Some(id) = from_name {
            if let Some((_, pre)) = converted.iter_mut().find(|(k, _)| k == id) {
                let amount = as_double(pre.get("amount"), 0.0) + as_double(m.get("amount"), 0.0);
                set(pre, "amount", Tag::Double(amount.to_bits()));
            } else {
                set(&mut m, "id", string(id));
                remove(&mut m, "uuid");
                remove(&mut m, "name");
                put(&mut converted, id.to_owned(), m);
            }
        } else {
            let id = format!("minecraft:{}", uuid.as_deref().unwrap_or("unknown"));
            set(&mut m, "id", string(&id));
            remove(&mut m, "uuid");
            remove(&mut m, "name");
            put(&mut converted, id, m);
        }
    }
    converted.into_iter().map(|(_, v)| v).collect()
}

fn attribute_modifier_ids(cx: &mut Cx) {
    cx.each(Ty::ItemStack, |s| {
        if let Some(c) = s.get_mut("components")
            && let Some(a) = c.get_mut("minecraft:attribute_modifiers")
            && let Some(m) = a.get("modifiers")
            && is_list(Some(m))
        {
            let fixed = fix_modifiers(stream(Some(m)));
            set(a, "modifiers", Tag::List(fixed));
        }
    });
    let fix_entity = |e: &mut Tag| {
        rename_and_fix(e, "Attributes", "attributes", |list| {
            if !is_list(Some(&list)) {
                return Some(list);
            }
            Some(Tag::List(
                stream(Some(&list))
                    .into_iter()
                    .map(|mut a| {
                        rename(&mut a, "Name", "id");
                        rename(&mut a, "Base", "base");
                        rename_and_fix(&mut a, "Modifiers", "modifiers", |mods| {
                            if !is_list(Some(&mods)) {
                                return Some(mods);
                            }
                            let converted = stream(Some(&mods))
                                .into_iter()
                                .map(|mut m| {
                                    rename(&mut m, "UUID", "uuid");
                                    rename(&mut m, "Name", "name");
                                    rename(&mut m, "Amount", "amount");
                                    rename_and_fix(&mut m, "Operation", "operation", |op| {
                                        Some(string(match as_int(Some(&op), 0) {
                                            0 => "add_value",
                                            1 => "add_multiplied_base",
                                            2 => "add_multiplied_total",
                                            _ => "invalid",
                                        }))
                                    });
                                    m
                                })
                                .collect();
                            Some(Tag::List(fix_modifiers(converted)))
                        });
                        a
                    })
                    .collect(),
            ))
        });
    };
    cx.each(Ty::Entity, fix_entity);
    cx.each(Ty::Player, fix_entity);
}

fn jukebox_ticks(cx: &mut Cx) {
    cx.block_entity("minecraft:jukebox", |be| {
        let ticks =
            as_long(be.get("TickCount"), 0).wrapping_sub(as_long(be.get("RecordStartTick"), 0));
        remove(be, "IsPlaying");
        remove(be, "TickCount");
        remove(be, "RecordStartTick");
        if ticks > 0 {
            set(be, "ticks_since_song_started", Tag::Long(ticks));
        }
    });
}

// ------------------------------------------------------------------ 4054, 4055

fn ominous_banner_rarity(cx: &mut Cx) {
    let fix = |c: &mut Tag| {
        if c.get("minecraft:item_name")
            .and_then(translation_key)
            .as_deref()
            == Some("block.minecraft.ominous_banner")
        {
            set(
                c,
                "minecraft:item_name",
                Tag::String(crate::nbt::JavaString(
                    super::json::translatable_component_json("block.minecraft.ominous_banner"),
                )),
            );
            set(c, "minecraft:rarity", string("uncommon"));
        }
    };
    cx.block_entity("minecraft:banner", |be| {
        if let Some(c) = be.get_mut("components") {
            fix(c);
        }
    });
    cx.item(
        |id| id == "minecraft:white_banner",
        |s| {
            if let Some(c) = s.get_mut("components") {
                fix(c);
            }
        },
    );
}

fn attribute_id_prefix(id: &str) -> Option<String> {
    let namespaced_id = namespaced(id);
    for prefix in ["generic.", "horse.", "player.", "zombie."] {
        let p = namespaced(prefix);
        if let Some(rest) = namespaced_id.strip_prefix(&p) {
            return Some(format!("minecraft:{rest}"));
        }
    }
    None
}

/// `AttributesRenameFix`.
pub(super) fn attributes_rename(
    cx: &mut Cx,
    old_component_format: bool,
    rename_fn: &dyn Fn(&str) -> Option<String>,
) {
    let fix_field = |v: &mut Tag, field: &str| {
        if let Some(s) = as_str(v.get(field))
            && let Some(new) = rename_fn(&s)
        {
            set(v, field, string(&new));
        }
    };
    cx.each(Ty::DataComponents, |c| {
        let Some(a) = c.get_mut("minecraft:attribute_modifiers") else {
            return;
        };
        let list = if old_component_format {
            a.get_mut("modifiers")
        } else {
            Some(a)
        };
        if let Some(Tag::List(mods)) = list {
            for m in mods {
                fix_field(m, "type");
            }
        }
    });
    let fix_entity = |e: &mut Tag| {
        if let Some(Tag::List(attrs)) = e.get_mut("attributes") {
            for a in attrs {
                fix_field(a, "id");
            }
        }
    };
    cx.each(Ty::Entity, fix_entity);
    cx.each(Ty::Player, fix_entity);
}

// ------------------------------------------------------------------ 4059, 4061

fn food_to_consumable(cx: &mut Cx) {
    cx.each(Ty::DataComponents, |c| {
        let Some(food) = c.get("minecraft:food").cloned() else {
            return;
        };
        let eat_seconds = as_float(food.get("eat_seconds"), 1.6);
        let effects: Vec<Tag> = stream(food.get("effects"))
            .into_iter()
            .map(|e| {
                Tag::compound([
                    ("type", string("minecraft:apply_effects")),
                    (
                        "effects",
                        Tag::List(e.get("effect").cloned().into_iter().collect()),
                    ),
                    (
                        "probability",
                        Tag::Float(as_float(e.get("probability"), 1.0).to_bits()),
                    ),
                ])
            })
            .collect();
        super::dynamic::copy_field(&food, "using_converts_to", c, "minecraft:use_remainder");
        let mut food = food;
        remove(&mut food, "eat_seconds");
        remove(&mut food, "effects");
        remove(&mut food, "using_converts_to");
        set(c, "minecraft:food", food);
        set(
            c,
            "minecraft:consumable",
            Tag::compound([
                ("consume_seconds", Tag::Float(eat_seconds.to_bits())),
                ("on_consume_effects", Tag::List(effects)),
            ]),
        );
    });
}

const TRIAL_CONFIGS: &[(&str, &str, &str)] = &[
    (
        "trial_chamber/breeze",
        "{simultaneous_mobs: 1.0f, simultaneous_mobs_added_per_player: 0.5f, spawn_potentials: [{data: {entity: {id: \"minecraft:breeze\"}}, weight: 1}], ticks_between_spawn: 20, total_mobs: 2.0f, total_mobs_added_per_player: 1.0f}",
        "{loot_tables_to_eject: [{data: \"minecraft:spawners/ominous/trial_chamber/key\", weight: 3}, {data: \"minecraft:spawners/ominous/trial_chamber/consumables\", weight: 7}], simultaneous_mobs: 2.0f, total_mobs: 4.0f}",
    ),
    (
        "trial_chamber/melee/husk",
        "{simultaneous_mobs: 3.0f, simultaneous_mobs_added_per_player: 0.5f, spawn_potentials: [{data: {entity: {id: \"minecraft:husk\"}}, weight: 1}], ticks_between_spawn: 20}",
        "{loot_tables_to_eject: [{data: \"minecraft:spawners/ominous/trial_chamber/key\", weight: 3}, {data: \"minecraft:spawners/ominous/trial_chamber/consumables\", weight: 7}], spawn_potentials: [{data: {entity: {id: \"minecraft:husk\"}, equipment: {loot_table: \"minecraft:equipment/trial_chamber_melee\", slot_drop_chances: 0.0f}}, weight: 1}]}",
    ),
    (
        "trial_chamber/melee/spider",
        "{simultaneous_mobs: 3.0f, simultaneous_mobs_added_per_player: 0.5f, spawn_potentials: [{data: {entity: {id: \"minecraft:spider\"}}, weight: 1}], ticks_between_spawn: 20}",
        "{loot_tables_to_eject: [{data: \"minecraft:spawners/ominous/trial_chamber/key\", weight: 3}, {data: \"minecraft:spawners/ominous/trial_chamber/consumables\", weight: 7}],simultaneous_mobs: 4.0f, total_mobs: 12.0f}",
    ),
    (
        "trial_chamber/melee/zombie",
        "{simultaneous_mobs: 3.0f, simultaneous_mobs_added_per_player: 0.5f, spawn_potentials: [{data: {entity: {id: \"minecraft:zombie\"}}, weight: 1}], ticks_between_spawn: 20}",
        "{loot_tables_to_eject: [{data: \"minecraft:spawners/ominous/trial_chamber/key\", weight: 3}, {data: \"minecraft:spawners/ominous/trial_chamber/consumables\", weight: 7}],spawn_potentials: [{data: {entity: {id: \"minecraft:zombie\"}, equipment: {loot_table: \"minecraft:equipment/trial_chamber_melee\", slot_drop_chances: 0.0f}}, weight: 1}]}",
    ),
    (
        "trial_chamber/ranged/poison_skeleton",
        "{simultaneous_mobs: 3.0f, simultaneous_mobs_added_per_player: 0.5f, spawn_potentials: [{data: {entity: {id: \"minecraft:bogged\"}}, weight: 1}], ticks_between_spawn: 20}",
        "{loot_tables_to_eject: [{data: \"minecraft:spawners/ominous/trial_chamber/key\", weight: 3}, {data: \"minecraft:spawners/ominous/trial_chamber/consumables\", weight: 7}],spawn_potentials: [{data: {entity: {id: \"minecraft:bogged\"}, equipment: {loot_table: \"minecraft:equipment/trial_chamber_ranged\", slot_drop_chances: 0.0f}}, weight: 1}]}",
    ),
    (
        "trial_chamber/ranged/skeleton",
        "{simultaneous_mobs: 3.0f, simultaneous_mobs_added_per_player: 0.5f, spawn_potentials: [{data: {entity: {id: \"minecraft:skeleton\"}}, weight: 1}], ticks_between_spawn: 20}",
        "{loot_tables_to_eject: [{data: \"minecraft:spawners/ominous/trial_chamber/key\", weight: 3}, {data: \"minecraft:spawners/ominous/trial_chamber/consumables\", weight: 7}], spawn_potentials: [{data: {entity: {id: \"minecraft:skeleton\"}, equipment: {loot_table: \"minecraft:equipment/trial_chamber_ranged\", slot_drop_chances: 0.0f}}, weight: 1}]}",
    ),
    (
        "trial_chamber/ranged/stray",
        "{simultaneous_mobs: 3.0f, simultaneous_mobs_added_per_player: 0.5f, spawn_potentials: [{data: {entity: {id: \"minecraft:stray\"}}, weight: 1}], ticks_between_spawn: 20}",
        "{loot_tables_to_eject: [{data: \"minecraft:spawners/ominous/trial_chamber/key\", weight: 3}, {data: \"minecraft:spawners/ominous/trial_chamber/consumables\", weight: 7}], spawn_potentials: [{data: {entity: {id: \"minecraft:stray\"}, equipment: {loot_table: \"minecraft:equipment/trial_chamber_ranged\", slot_drop_chances: 0.0f}}, weight: 1}]}",
    ),
    (
        "trial_chamber/slow_ranged/poison_skeleton",
        "{simultaneous_mobs: 4.0f, simultaneous_mobs_added_per_player: 2.0f, spawn_potentials: [{data: {entity: {id: \"minecraft:bogged\"}}, weight: 1}], ticks_between_spawn: 160}",
        "{loot_tables_to_eject: [{data: \"minecraft:spawners/ominous/trial_chamber/key\", weight: 3}, {data: \"minecraft:spawners/ominous/trial_chamber/consumables\", weight: 7}], spawn_potentials: [{data: {entity: {id: \"minecraft:bogged\"}, equipment: {loot_table: \"minecraft:equipment/trial_chamber_ranged\", slot_drop_chances: 0.0f}}, weight: 1}]}",
    ),
    (
        "trial_chamber/slow_ranged/skeleton",
        "{simultaneous_mobs: 4.0f, simultaneous_mobs_added_per_player: 2.0f, spawn_potentials: [{data: {entity: {id: \"minecraft:skeleton\"}}, weight: 1}], ticks_between_spawn: 160}",
        "{loot_tables_to_eject: [{data: \"minecraft:spawners/ominous/trial_chamber/key\", weight: 3}, {data: \"minecraft:spawners/ominous/trial_chamber/consumables\", weight: 7}], spawn_potentials: [{data: {entity: {id: \"minecraft:skeleton\"}, equipment: {loot_table: \"minecraft:equipment/trial_chamber_ranged\", slot_drop_chances: 0.0f}}, weight: 1}]}",
    ),
    (
        "trial_chamber/slow_ranged/stray",
        "{simultaneous_mobs: 4.0f, simultaneous_mobs_added_per_player: 2.0f, spawn_potentials: [{data: {entity: {id: \"minecraft:stray\"}}, weight: 1}], ticks_between_spawn: 160}",
        "{loot_tables_to_eject: [{data: \"minecraft:spawners/ominous/trial_chamber/key\", weight: 3}, {data: \"minecraft:spawners/ominous/trial_chamber/consumables\", weight: 7}],spawn_potentials: [{data: {entity: {id: \"minecraft:stray\"}, equipment: {loot_table: \"minecraft:equipment/trial_chamber_ranged\", slot_drop_chances: 0.0f}}, weight: 1}]}",
    ),
    (
        "trial_chamber/small_melee/baby_zombie",
        "{simultaneous_mobs: 2.0f, simultaneous_mobs_added_per_player: 0.5f, spawn_potentials: [{data: {entity: {IsBaby: 1b, id: \"minecraft:zombie\"}}, weight: 1}], ticks_between_spawn: 20}",
        "{loot_tables_to_eject: [{data: \"minecraft:spawners/ominous/trial_chamber/key\", weight: 3}, {data: \"minecraft:spawners/ominous/trial_chamber/consumables\", weight: 7}], spawn_potentials: [{data: {entity: {IsBaby: 1b, id: \"minecraft:zombie\"}, equipment: {loot_table: \"minecraft:equipment/trial_chamber_melee\", slot_drop_chances: 0.0f}}, weight: 1}]}",
    ),
    (
        "trial_chamber/small_melee/cave_spider",
        "{simultaneous_mobs: 3.0f, simultaneous_mobs_added_per_player: 0.5f, spawn_potentials: [{data: {entity: {id: \"minecraft:cave_spider\"}}, weight: 1}], ticks_between_spawn: 20}",
        "{loot_tables_to_eject: [{data: \"minecraft:spawners/ominous/trial_chamber/key\", weight: 3}, {data: \"minecraft:spawners/ominous/trial_chamber/consumables\", weight: 7}], simultaneous_mobs: 4.0f, total_mobs: 12.0f}",
    ),
    (
        "trial_chamber/small_melee/silverfish",
        "{simultaneous_mobs: 3.0f, simultaneous_mobs_added_per_player: 0.5f, spawn_potentials: [{data: {entity: {id: \"minecraft:silverfish\"}}, weight: 1}], ticks_between_spawn: 20}",
        "{loot_tables_to_eject: [{data: \"minecraft:spawners/ominous/trial_chamber/key\", weight: 3}, {data: \"minecraft:spawners/ominous/trial_chamber/consumables\", weight: 7}], simultaneous_mobs: 4.0f, total_mobs: 12.0f}",
    ),
    (
        "trial_chamber/small_melee/slime",
        "{simultaneous_mobs: 3.0f, simultaneous_mobs_added_per_player: 0.5f, spawn_potentials: [{data: {entity: {Size: 1, id: \"minecraft:slime\"}}, weight: 3}, {data: {entity: {Size: 2, id: \"minecraft:slime\"}}, weight: 1}], ticks_between_spawn: 20}",
        "{loot_tables_to_eject: [{data: \"minecraft:spawners/ominous/trial_chamber/key\", weight: 3}, {data: \"minecraft:spawners/ominous/trial_chamber/consumables\", weight: 7}], simultaneous_mobs: 4.0f, total_mobs: 12.0f}",
    ),
];

fn snbt(s: &str) -> Tag {
    crate::nbt::parse_snbt(s)
        .map(|(t, _)| t)
        .unwrap_or_else(|_| empty_map())
}
/// `CompoundTag.merge`.
fn merge(into: &mut Tag, other: &Tag) {
    let (Tag::Compound(a), Tag::Compound(b)) = (into, other) else {
        return;
    };
    for (k, v) in b {
        match (a.get_mut(k), v) {
            (Some(existing @ Tag::Compound(_)), Tag::Compound(_)) => merge(existing, v),
            _ => {
                a.insert(k.clone(), v.clone());
            }
        }
    }
}
fn remove_trial_defaults(tag: &mut Tag) {
    let int = |t: &Tag, k: &str| as_int(t.get(k), 0);
    let float = |t: &Tag, k: &str| as_float(t.get(k), 0.0);
    if int(tag, "spawn_range") == 4 {
        remove(tag, "spawn_range");
    }
    if float(tag, "total_mobs") == 6.0 {
        remove(tag, "total_mobs");
    }
    if float(tag, "simultaneous_mobs") == 2.0 {
        remove(tag, "simultaneous_mobs");
    }
    if float(tag, "total_mobs_added_per_player") == 2.0 {
        remove(tag, "total_mobs_added_per_player");
    }
    if float(tag, "simultaneous_mobs_added_per_player") == 1.0 {
        remove(tag, "simultaneous_mobs_added_per_player");
    }
    if int(tag, "ticks_between_spawn") == 40 {
        remove(tag, "ticks_between_spawn");
    }
}

fn trial_config_key(normal: &Tag, ominous: &Tag) -> Option<&'static str> {
    TRIAL_CONFIGS.iter().find_map(|(name, n, o)| {
        let n = snbt(n);
        if *normal != n {
            return None;
        }
        let o = snbt(o);
        let mut merged = n.clone();
        merge(&mut merged, &o);
        let mut trimmed = merged.clone();
        remove_trial_defaults(&mut trimmed);
        (*ominous == o || *ominous == merged || *ominous == trimmed).then_some(*name)
    })
}

fn trial_spawner_config_in_registry(cx: &mut Cx) {
    cx.block_entity("minecraft:trial_spawner", |be| {
        let (Some(normal), Some(ominous)) = (be.get("normal_config"), be.get("ominous_config"))
        else {
            return;
        };
        if let Some(name) = trial_config_key(normal, ominous) {
            set(
                be,
                "normal_config",
                string(&format!("minecraft:{name}/normal")),
            );
            set(
                be,
                "ominous_config",
                string(&format!("minecraft:{name}/ominous")),
            );
        }
    });
}

// ------------------------------------------------------------------ 4067, 4068

fn boat_split(cx: &mut Cx) {
    cx.each(Ty::Entity, |e| {
        let id = namespaced(&as_str(e.get("id")).unwrap_or_default());
        let chest = id == "minecraft:chest_boat";
        if !chest && id != "minecraft:boat" {
            return;
        }
        let wood = as_str(e.get("Type"));
        let new = match wood.as_deref() {
            Some("spruce") => "spruce",
            Some("birch") => "birch",
            Some("jungle") => "jungle",
            Some("acacia") => "acacia",
            Some("cherry") => "cherry",
            Some("dark_oak") => "dark_oak",
            Some("mangrove") => "mangrove",
            Some("bamboo") => "bamboo",
            _ => "oak",
        };
        let new_id = match (new, chest) {
            ("bamboo", false) => "minecraft:bamboo_raft".into(),
            ("bamboo", true) => "minecraft:bamboo_chest_raft".into(),
            (w, false) => format!("minecraft:{w}_boat"),
            (w, true) => format!("minecraft:{w}_chest_boat"),
        };
        // The remainder update runs on the value cast to the new type, whose choice
        // no longer has the old boat id: it finds nothing, so Type stays.
        set(e, "id", string(&new_id));
    });
}

/// `LockComponentPredicateFix.fixLock`.
fn fix_lock(v: Tag) -> Option<Tag> {
    let name = as_str(Some(&v))?;
    if name.is_empty() {
        return None;
    }
    let escaped = name.replace('\\', "\\\\").replace('"', "\\\"");
    Some(Tag::compound([(
        "components",
        Tag::compound([("minecraft:custom_name", string(&format!("\"{escaped}\"")))]),
    )]))
}

/// `InvalidLockComponentFix.isBrokenLock`: `{components: {custom_name: '""'}}`.
fn broken_lock(v: &Tag) -> bool {
    let one = |t: &Tag, field: &str| {
        matches!(t, Tag::Compound(c) if c.len() == 1) && t.get(field).is_some()
    };
    one(v, "components")
        && v.get("components").is_some_and(|c| {
            one(c, "minecraft:custom_name")
                && as_str(c.get("minecraft:custom_name")).as_deref() == Some("\"\"")
        })
}

// ------------------------------------------------------------------ 4181, 4187

fn furnace_burn_time(cx: &mut Cx, id: &str) {
    cx.block_entity(id, |be| {
        rename(be, "CookTime", "cooking_time_spent");
        rename(be, "CookTimeTotal", "cooking_total_time");
        rename(be, "BurnTime", "lit_time_remaining");
        if let Some(v) = be.get("lit_time_remaining").cloned() {
            set(be, "lit_total_time", v);
        }
    });
}

fn base_value(cx: &mut Cx, entity: &str, attribute: &str, from: f64, to: f64) {
    cx.entity(entity, |e| {
        update(e, "attributes", |list| {
            Tag::List(
                stream(Some(&list))
                    .into_iter()
                    .map(|mut a| {
                        if namespaced(&as_string(a.get("id"), "")) == attribute {
                            let base = as_double(a.get("base"), 0.0);
                            let new = if base == from { to } else { base };
                            set(&mut a, "base", Tag::Double(new.to_bits()));
                        }
                        a
                    })
                    .collect(),
            )
        });
    });
}

// ------------------------------------------------------------------ 4293, 4294, 4297

fn drop_chances_format(cx: &mut Cx) {
    cx.each(Ty::Entity, |e| {
        let parse = |v: Option<&Tag>| -> Vec<f32> {
            stream(v).iter().map(|t| as_float(Some(t), 0.085)).collect()
        };
        let armor = parse(e.get("ArmorDropChances"));
        let hands = parse(e.get("HandDropChances"));
        let body = e
            .get("body_armor_drop_chance")
            .and_then(as_number)
            .map_or(0.085, |n| n.float_value());
        remove(e, "ArmorDropChances");
        remove(e, "HandDropChances");
        remove(e, "body_armor_drop_chance");
        let mut chances = empty_map();
        for (slots, values) in [
            (&["feet", "legs", "chest", "head"][..], &armor),
            (&["mainhand", "offhand"][..], &hands),
        ] {
            for (slot, chance) in slots.iter().zip(values.iter()) {
                if *chance != 0.085 {
                    set(&mut chances, slot, Tag::Float(chance.to_bits()));
                }
            }
        }
        if body != 0.085 {
            set(&mut chances, "body", Tag::Float(body.to_bits()));
        }
        if !is_empty_map(&chances) {
            set(e, "drop_chances", chances);
        }
    });
}

/// `LegacyBlockPropertyRenameAndFix`.
fn block_property_rename(
    cx: &mut Cx,
    block: &str,
    old: &str,
    new: &str,
    f: &dyn Fn(&str) -> String,
) {
    cx.each(Ty::BlockState, |s| {
        if as_str(s.get("Name")).map(|n| namespaced(&n)).as_deref() != Some(block) {
            return;
        }
        if let Some(props) = s.get_mut("Properties") {
            rename_and_fix(props, old, new, |v| {
                Some(string(&f(&as_string(Some(&v), ""))))
            });
        }
    });
}

fn forced_chunk_to_ticket(cx: &mut Cx) {
    cx.each(Ty::SavedTickets, |v| {
        if let Some(data) = v.get_mut("data") {
            rename_and_fix(data, "Forced", "tickets", |forced| {
                let longs: Vec<i64> = match &forced {
                    Tag::LongArray(a) => a.clone(),
                    other => stream(Some(other))
                        .iter()
                        .map(|t| as_long(Some(t), 0))
                        .collect(),
                };
                Some(Tag::List(
                    longs
                        .into_iter()
                        .map(|l| {
                            Tag::compound([
                                ("type", string("minecraft:forced")),
                                ("level", Tag::Int(31)),
                                ("ticks_left", Tag::Long(0)),
                                ("chunk_pos", Tag::Long(l)),
                            ])
                        })
                        .collect(),
                ))
            });
        }
    });
}

// ------------------------------------------------------------------ 4299

fn entity_spawner_item_variants(cx: &mut Cx) {
    cx.each(Ty::ItemStack, |s| {
        let id = namespaced(&as_str(s.get("id")).unwrap_or_default());
        let Some(c) = s.get_mut("components") else {
            return;
        };
        if id == "minecraft:painting" {
            fix_painting(c);
            return;
        }
        let Some(bucket) = c.get("minecraft:bucket_entity_data").cloned() else {
            return;
        };
        match id.as_str() {
            "minecraft:salmon_bucket" => {
                if let Some(ty) = bucket.get("type").cloned() {
                    update(c, "minecraft:bucket_entity_data", |mut b| {
                        remove(&mut b, "type");
                        b
                    });
                    set(c, "minecraft:salmon/size", ty);
                }
            }
            "minecraft:axolotl_bucket" => {
                if let Some(v) = bucket.get("Variant").and_then(as_number) {
                    let variant = match v.int_value() {
                        1 => "wild",
                        2 => "gold",
                        3 => "cyan",
                        4 => "blue",
                        _ => "lucy",
                    };
                    update(c, "minecraft:bucket_entity_data", |mut b| {
                        remove(&mut b, "Variant");
                        b
                    });
                    set(c, "minecraft:axolotl/variant", string(variant));
                }
            }
            "minecraft:tropical_fish_bucket" => {
                if let Some(v) = bucket.get("BucketVariantTag").and_then(as_number) {
                    let packed = v.int_value();
                    let pattern = match packed & 0xFFFF {
                        1 => "flopper",
                        256 => "sunstreak",
                        257 => "stripey",
                        512 => "snooper",
                        513 => "glitter",
                        768 => "dasher",
                        769 => "blockfish",
                        1024 => "brinely",
                        1025 => "betty",
                        1280 => "spotty",
                        1281 => "clayfish",
                        _ => "kob",
                    };
                    update(c, "minecraft:bucket_entity_data", |mut b| {
                        remove(&mut b, "BucketVariantTag");
                        b
                    });
                    set(c, "minecraft:tropical_fish/pattern", string(pattern));
                    set(
                        c,
                        "minecraft:tropical_fish/base_color",
                        string(dye_color(packed >> 16 & 0xFF)),
                    );
                    set(
                        c,
                        "minecraft:tropical_fish/pattern_color",
                        string(dye_color(packed >> 24 & 0xFF)),
                    );
                }
            }
            _ => {}
        }
    });
}

fn fix_painting(c: &mut Tag) {
    let Some(data) = c.get("minecraft:entity_data").cloned() else {
        return;
    };
    if as_str(data.get("id")).as_deref() != Some("minecraft:painting") {
        return;
    }
    let variant = data.get("variant").cloned();
    let mut rest = data;
    remove(&mut rest, "variant");
    let mut without_id = rest.clone();
    remove(&mut without_id, "id");
    if is_empty_map(&without_id) {
        remove(c, "minecraft:entity_data");
    } else {
        set(c, "minecraft:entity_data", rest);
    }
    if let Some(v) = variant {
        set(c, "minecraft:painting/variant", v);
    }
}

// ------------------------------------------------------------------ 4300, 4301, 4303

fn saddle_drop_chance(e: &mut Tag) {
    let mut chances = or_empty_map(e.get("drop_chances"));
    set(&mut chances, "saddle", Tag::Float(2.0f32.to_bits()));
    set(e, "drop_chances", chances);
}

fn saddle_equipment_slot(cx: &mut Cx) {
    let k = cx.k;
    cx.each(Ty::Entity, |e| {
        let id = namespaced(&as_str(e.get("id")).unwrap_or_default());
        match id.as_str() {
            "minecraft:horse"
            | "minecraft:skeleton_horse"
            | "minecraft:zombie_horse"
            | "minecraft:donkey"
            | "minecraft:mule"
            | "minecraft:camel"
            | "minecraft:llama"
            | "minecraft:trader_llama" => {
                if e.get("SaddleItem").is_some() {
                    rename(e, "SaddleItem", "saddle");
                    saddle_drop_chance(e);
                }
                reread(k, e);
            }
            "minecraft:pig" | "minecraft:strider" => {
                let saddled = as_bool(e.get("Saddle"), false);
                remove(e, "Saddle");
                if saddled {
                    set(
                        e,
                        "saddle",
                        Tag::compound([("id", string("minecraft:saddle")), ("count", Tag::Int(1))]),
                    );
                    saddle_drop_chance(e);
                }
                reread(k, e);
            }
            _ => {}
        }
    });
}

/// `EquipmentFormatFix` over an entity's (or player's) equipment fields.
pub(super) fn equipment_to_slots(e: &mut Tag) {
    let armor = match remove(e, "ArmorItems") {
        Some(Tag::List(l)) => l,
        _ => Vec::new(),
    };
    let hands = match remove(e, "HandItems") {
        Some(Tag::List(l)) => l,
        _ => Vec::new(),
    };
    let body = remove(e, "body_armor_item");
    let saddle = remove(e, "saddle");
    // A stack without an id (an id that does not read as an item name is none) is a
    // placeholder for an empty slot.
    let pick = |list: &Vec<Tag>, i: usize| {
        list.get(i)
            .filter(|s| matches!(s.get("id"), Some(Tag::String(_))))
            .cloned()
    };
    let slots = [
        ("mainhand", pick(&hands, 0)),
        ("offhand", pick(&hands, 1)),
        ("feet", pick(&armor, 0)),
        ("legs", pick(&armor, 1)),
        ("chest", pick(&armor, 2)),
        ("head", pick(&armor, 3)),
        ("body", body),
        ("saddle", saddle),
    ];
    // The old fields are typed: they come back from the entity's remainder.
    for field in ["ArmorItems", "HandItems", "body_armor_item", "saddle"] {
        super::drop_typed(e, field);
    }
    if slots.iter().all(|(_, s)| s.is_none()) {
        return;
    }
    let mut eq = Compound::new();
    for (name, stack) in slots {
        if let Some(s) = stack {
            eq.insert(name.into(), s);
        }
    }
    set(e, "equipment", Tag::Compound(eq));
}

fn equipment_format(cx: &mut Cx) {
    cx.each(Ty::Entity, equipment_to_slots);
}

fn fall_distance_to_double(e: &mut Tag) {
    rename_and_fix(e, "FallDistance", "fall_distance", |f| {
        Some(Tag::Double(f64::from(as_float(Some(&f), 0.0)).to_bits()))
    });
}

// ------------------------------------------------------------------ 4306, 4307

fn thrown_potion_split(cx: &mut Cx) {
    cx.each(Ty::Entity, |e| {
        if !id_is(e, "minecraft:potion") {
            return;
        }
        let item = e
            .get("Item")
            .and_then(|i| as_str(i.get("id")))
            .map(|i| namespaced(&i))
            .unwrap_or_default();
        let new = if item == "minecraft:lingering_potion" {
            "minecraft:lingering_potion"
        } else {
            "minecraft:splash_potion"
        };
        set(e, "id", string(new));
    });
}

const ADDITIONAL_TOOLTIP_TYPES: [&str; 17] = [
    "minecraft:banner_patterns",
    "minecraft:bees",
    "minecraft:block_entity_data",
    "minecraft:block_state",
    "minecraft:bundle_contents",
    "minecraft:charged_projectiles",
    "minecraft:container",
    "minecraft:container_loot",
    "minecraft:firework_explosion",
    "minecraft:fireworks",
    "minecraft:instrument",
    "minecraft:map_id",
    "minecraft:painting/variant",
    "minecraft:pot_decorations",
    "minecraft:potion_contents",
    "minecraft:tropical_fish/pattern",
    "minecraft:written_book_content",
];

fn tooltip_display(cx: &mut Cx) {
    cx.each(Ty::DataComponents, |c| {
        let mut hidden: Vec<String> = Vec::new();
        let add = |h: &mut Vec<String>, id: &str| {
            if !h.iter().any(|x| x == id) {
                h.push(id.to_owned());
            }
        };
        for id in ["minecraft:can_place_on", "minecraft:can_break"] {
            if let Some(p) = c.get_mut(id)
                && let Some(predicates) = p.get("predicates").cloned()
            {
                if !as_bool(p.get("show_in_tooltip"), true) {
                    add(&mut hidden, id);
                }
                *p = predicates;
            }
        }
        let fix_component = |c: &mut Tag, h: &mut Vec<String>, id: &str, unwrap: Option<&str>| {
            update(c, id, |mut component| {
                if !as_bool(component.get("show_in_tooltip"), true) {
                    add(h, id);
                }
                remove(&mut component, "show_in_tooltip");
                match unwrap.and_then(|f| component.get(f).cloned()) {
                    Some(inner) => inner,
                    None => component,
                }
            });
        };
        fix_component(c, &mut hidden, "minecraft:trim", None);
        fix_component(c, &mut hidden, "minecraft:unbreakable", None);
        fix_component(c, &mut hidden, "minecraft:dyed_color", Some("rgb"));
        fix_component(
            c,
            &mut hidden,
            "minecraft:attribute_modifiers",
            Some("modifiers"),
        );
        fix_component(c, &mut hidden, "minecraft:enchantments", Some("levels"));
        fix_component(
            c,
            &mut hidden,
            "minecraft:stored_enchantments",
            Some("levels"),
        );
        fix_component(c, &mut hidden, "minecraft:jukebox_playable", Some("song"));
        let hide_tooltip = remove(c, "minecraft:hide_tooltip").is_some();
        let hide_additional = remove(c, "minecraft:hide_additional_tooltip").is_some();
        if hide_additional {
            for id in ADDITIONAL_TOOLTIP_TYPES {
                if c.get(id).is_some() && !hidden.iter().any(|x| x == id) {
                    hidden.push(id.to_owned());
                }
            }
        }
        if !hidden.is_empty() || hide_tooltip {
            // A HashSet: its iteration order.
            let hidden = java_hash_order(hidden, String::clone);
            set(
                c,
                "minecraft:tooltip_display",
                Tag::compound([
                    ("hide_tooltip", Tag::Byte(i8::from(hide_tooltip))),
                    (
                        "hidden_components",
                        Tag::List(hidden.iter().map(|s| string(s)).collect()),
                    ),
                ]),
            );
        }
    });
}

// ------------------------------------------------------------------ 4309, 4311, 4312, 4314

fn raid_renames(cx: &mut Cx) {
    cx.each(Ty::SavedRaids, |v| {
        if let Some(d) = v.get_mut("data") {
            rename_and_fix(d, "Raids", "raids", |raids| {
                Some(Tag::List(
                    stream(Some(&raids))
                        .into_iter()
                        .map(|mut r| {
                            fix_inline_block_pos(&mut r, "CX", "CY", "CZ", "center");
                            for (a, b) in [
                                ("Id", "id"),
                                ("Started", "started"),
                                ("Active", "active"),
                                ("TicksActive", "ticks_active"),
                                ("BadOmenLevel", "raid_omen_level"),
                                ("GroupsSpawned", "groups_spawned"),
                                ("PreRaidTicks", "cooldown_ticks"),
                                ("PostRaidTicks", "post_raid_ticks"),
                                ("TotalHealth", "total_health"),
                                ("NumGroups", "group_count"),
                                ("Status", "status"),
                                ("HeroesOfTheVillage", "heroes_of_the_village"),
                            ] {
                                rename(&mut r, a, b);
                            }
                            r
                        })
                        .collect(),
                ))
            });
            rename(d, "Tick", "tick");
            rename(d, "NextAvailableID", "next_id");
        }
    });
}

fn chunk_ticket_unpack_pos(cx: &mut Cx) {
    cx.each(Ty::SavedTickets, |v| {
        if let Some(d) = v.get_mut("data") {
            update(d, "tickets", |tickets| {
                Tag::List(
                    stream(Some(&tickets))
                        .into_iter()
                        .map(|mut t| {
                            update(&mut t, "chunk_pos", |p| {
                                let key = as_long(Some(&p), 0);
                                Tag::IntArray(vec![key as i32, (key as u64 >> 32) as i32])
                            });
                            t
                        })
                        .collect(),
                )
            });
        }
    });
}

/// `AdvancementsRenameFix`: the advancement ids of an advancements file.
pub(super) fn advancements_rename(cx: &mut Cx, rename_fn: &dyn Fn(&str) -> Option<String>) {
    cx.each(Ty::Advancements, |v| {
        if let Tag::Compound(c) = v {
            let old = std::mem::take(c);
            for (k, value) in old {
                let id = k.to_string_lossy();
                let k = rename_fn(&id).map_or(k, |n| n.as_str().into());
                c.insert(k, value);
            }
        }
    });
}

fn player_equipment(cx: &mut Cx) {
    cx.each(Ty::Player, |p| {
        let mut equipment = Compound::new();
        update(p, "Inventory", |inv| {
            Tag::List(
                stream(Some(&inv))
                    .into_iter()
                    .filter_map(|mut item| {
                        let slot = match as_int(item.get("Slot"), -1) {
                            100 => Some("feet"),
                            101 => Some("legs"),
                            102 => Some("chest"),
                            103 => Some("head"),
                            -106 => Some("offhand"),
                            _ => None,
                        };
                        match slot {
                            Some(s) => {
                                remove(&mut item, "Slot");
                                equipment.insert(s.into(), item);
                                None
                            }
                            None => Some(item),
                        }
                    })
                    .collect(),
            )
        });
        set(p, "equipment", Tag::Compound(equipment));
    });
}

fn inline_block_pos_format(cx: &mut Cx) {
    cx.each(Ty::Player, |p| {
        fix_inline_block_pos(p, "SleepingX", "SleepingY", "SleepingZ", "sleeping_pos");
        let spawn = (
            p.get("SpawnX").and_then(as_number),
            p.get("SpawnY").and_then(as_number),
            p.get("SpawnZ").and_then(as_number),
        );
        if let (Some(x), Some(y), Some(z)) = spawn {
            let mut respawn = Tag::compound([(
                "pos",
                Tag::IntArray(vec![x.int_value(), y.int_value(), z.int_value()]),
            )]);
            let source = p.clone();
            super::dynamic::copy_field(&source, "SpawnAngle", &mut respawn, "angle");
            super::dynamic::copy_field(&source, "SpawnDimension", &mut respawn, "dimension");
            super::dynamic::copy_field(&source, "SpawnForced", &mut respawn, "forced");
            for k in [
                "SpawnX",
                "SpawnY",
                "SpawnZ",
                "SpawnAngle",
                "SpawnDimension",
                "SpawnForced",
            ] {
                remove(p, k);
            }
            set(p, "respawn", respawn);
        }
        if let Some(pos) = remove(p, "enteredNetherPosition") {
            set(
                p,
                "entered_nether_pos",
                Tag::List(
                    ["x", "y", "z"]
                        .iter()
                        .map(|k| Tag::Double(as_double(pos.get(k), 0.0).to_bits()))
                        .collect(),
                ),
            );
        }
    });
    cx.each(Ty::Entity, |e| {
        fix_inline_block_pos(e, "SleepingX", "SleepingY", "SleepingZ", "sleeping_pos");
        if id_is(e, "minecraft:vex") {
            rename(e, "LifeTicks", "life_ticks");
            fix_inline_block_pos(e, "BoundX", "BoundY", "BoundZ", "bound_pos");
        }
        if id_is(e, "minecraft:phantom") {
            rename(e, "Size", "size");
            fix_inline_block_pos(e, "AX", "AY", "AZ", "anchor_pos");
        }
        if id_is(e, "minecraft:turtle") {
            for k in ["TravelPosX", "TravelPosY", "TravelPosZ"] {
                remove(e, k);
            }
            fix_inline_block_pos(e, "HomePosX", "HomePosY", "HomePosZ", "home_pos");
            rename(e, "HasEgg", "has_egg");
        }
        for id in [
            "minecraft:item_frame",
            "minecraft:glow_item_frame",
            "minecraft:painting",
            "minecraft:leash_knot",
        ] {
            if id_is(e, id) {
                fix_inline_block_pos(e, "TileX", "TileY", "TileZ", "block_pos");
            }
        }
    });
}
