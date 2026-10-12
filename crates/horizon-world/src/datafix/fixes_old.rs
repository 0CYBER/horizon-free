//! Fixes of 1.19 to 1.20 (keys 3076 to 3459): what worlds saved by 1.18.2 to
//! 1.19.4 need before the fixes of [`super::fixes_a`]. The options fixes of
//! this range (3201, 3214, 3319) are client settings, not world data.
use super::dynamic::{
    as_number, as_str, is_list, namespaced, remove, rename, set, stream, string, update,
};
use super::fixes_a::fix_block_pos;
use super::fixes_b::feature_flag_remove;
use super::schema::{Key, Ty, key};
use super::{Cx, FixFn, id_is, renamer, reread_as};
use crate::nbt::{Compound, Tag};

pub(super) fn register(out: &mut Vec<(Key, FixFn)>) {
    out.push((key(3077, 0), chunk_delete_ignored_light));
    out.push((key(3084, 0), |cx| {
        cx.rename(Ty::GameEventName, renamer(GAME_EVENTS))
    }));
    out.push((key(3086, 0), |cx| {
        entity_variant(cx, "minecraft:cat", "CatType", cat_type)
    }));
    out.push((key(3086, 0), |cx| {
        criteria_rename(cx, "minecraft:husbandry/complete_catalogue", &|s| {
            CAT_TEXTURES
                .iter()
                .find(|(from, _)| *from == s)
                .map(|(_, to)| (*to).to_owned())
        });
    }));
    out.push((key(3087, 0), |cx| {
        entity_variant(cx, "minecraft:frog", "Variant", |n| match n {
            0 => Some("minecraft:temperate"),
            1 => Some("minecraft:warm"),
            2 => Some("minecraft:cold"),
            _ => None,
        })
    }));
    out.push((key(3090, 0), |cx| {
        cx.entity("minecraft:painting", |e| {
            rename(e, "Motive", "variant");
            rename(e, "Facing", "facing");
        })
    }));
    out.push((key(3093, 0), |cx| {
        cx.entity("minecraft:goat", |e| {
            set(e, "HasLeftHorn", Tag::Byte(1));
            set(e, "HasRightHorn", Tag::Byte(1));
        })
    }));
    out.push((key(3094, 0), goat_horn_id));
    out.push((key(3097, 0), filtered_books));
    out.push((key(3097, 0), filtered_signs));
    out.push((key(3097, 0), |cx| {
        cx.entity("minecraft:cat", |e| {
            if as_str(e.get("variant")).as_deref() == Some("minecraft:british") {
                set(e, "variant", string("minecraft:british_shorthair"));
            }
        })
    }));
    out.push((key(3097, 0), |cx| {
        criteria_rename(cx, "minecraft:husbandry/complete_catalogue", &|s| {
            (s == "minecraft:british").then(|| "minecraft:british_shorthair".to_owned())
        });
    }));
    out.push((key(3097, 0), |cx| {
        poi_records(cx, |r| {
            !matches!(
                as_str(r.get("type")).as_deref(),
                Some("minecraft:unemployed" | "minecraft:nitwit") | None
            )
        })
    }));
    out.push((key(3108, 0), |cx| {
        cx.each_out(Ty::Chunk, |c| {
            let dimension = c.get("__context").and_then(|x| as_str(x.get("dimension")));
            if dimension.as_deref() != Some("minecraft:overworld") {
                remove(c, "blending_data");
            }
        })
    }));
    out.push((key(3209, 0), pig_spawn_egg));
    out.push((key(3322, 0), effect_duration));
    out.push((key(3438, 0), |cx| {
        cx.each(Ty::BlockEntity, |be| {
            if id_is(be, "minecraft:suspicious_sand") {
                set(be, "id", string("minecraft:brushable_block"));
            }
        })
    }));
    out.push((key(3438, 0), |cx| {
        cx.each_out(Ty::BlockEntity, |be| {
            if id_is(be, "minecraft:brushable_block") {
                rename(be, "loot_table", "LootTable");
                rename(be, "loot_table_seed", "LootTableSeed");
            }
        })
    }));
    out.push((key(3438, 0), |cx| {
        cx.rename(Ty::ItemName, renamer(POTTERY_SHARDS))
    }));
    out.push((key(3439, 0), |cx| sign_double_sided(cx, "minecraft:sign")));
    out.push((key(3439, 1), |cx| {
        sign_double_sided(cx, "minecraft:hanging_sign")
    }));
    out.push((key(3440, 0), multi_noise_preset));
    out.push((key(3440, 0), |cx| {
        feature_flag_remove(cx, &["minecraft:update_1_20"])
    }));
    out.push((key(3447, 0), |cx| {
        cx.rename(Ty::ItemName, |id| {
            let id = namespaced(id);
            POTTERY_SHERDS
                .iter()
                .find(|(from, _)| *from == id)
                .map(|(_, to)| *to)
        })
    }));
    // DecoratedPotFieldRenameFix converts unchecked: the typed shards are written
    // as sherds, and the raw shards come back from the block entity's remainder.
    out.push((key(3448, 0), |cx| {
        cx.each(Ty::BlockEntity, |be| {
            if id_is(be, "minecraft:decorated_pot")
                && let Some(shards) = be.get("shards").cloned()
            {
                set(be, "sherds", shards);
                super::drop_typed(be, "shards");
            }
        })
    }));
    out.push((key(3450, 0), remap_chunk_status));
    out.push((key(3451, 0), chunk_delete_light));
    out.push((key(3459, 0), legacy_dragon_fight));
}

// ------------------------------------------------------------------ 3077, 3451

/// `ChunkDeleteIgnoredLightDataFix`.
fn chunk_delete_ignored_light(cx: &mut Cx) {
    cx.each(Ty::Chunk, |c| {
        let light_on = c
            .get("isLightOn")
            .and_then(as_number)
            .is_some_and(|n| n.double_value() != 0.0);
        if !light_on && let Some(Tag::List(sections)) = c.get_mut("sections") {
            for s in sections {
                remove(s, "BlockLight");
                remove(s, "SkyLight");
            }
        }
    });
}

/// `ChunkDeleteLightFix`.
fn chunk_delete_light(cx: &mut Cx) {
    cx.each(Ty::Chunk, |c| {
        remove(c, "isLightOn");
        if let Some(Tag::List(sections)) = c.get_mut("sections") {
            for s in sections {
                remove(s, "BlockLight");
                remove(s, "SkyLight");
            }
        }
    });
}

// ------------------------------------------------------------------ 3084

const GAME_EVENTS: &[(&str, &str)] = &[
    ("minecraft:block_press", "minecraft:block_activate"),
    ("minecraft:block_switch", "minecraft:block_activate"),
    ("minecraft:block_unpress", "minecraft:block_deactivate"),
    ("minecraft:block_unswitch", "minecraft:block_deactivate"),
    ("minecraft:drinking_finish", "minecraft:drink"),
    ("minecraft:elytra_free_fall", "minecraft:elytra_glide"),
    ("minecraft:entity_damaged", "minecraft:entity_damage"),
    ("minecraft:entity_dying", "minecraft:entity_die"),
    ("minecraft:entity_killed", "minecraft:entity_die"),
    ("minecraft:mob_interact", "minecraft:entity_interact"),
    ("minecraft:ravager_roar", "minecraft:entity_roar"),
    ("minecraft:ring_bell", "minecraft:block_change"),
    ("minecraft:shulker_close", "minecraft:container_close"),
    ("minecraft:shulker_open", "minecraft:container_open"),
    ("minecraft:wolf_shaking", "minecraft:entity_shake"),
];

// ------------------------------------------------------------------ 3086, 3087, 3097

fn cat_type(n: i32) -> Option<&'static str> {
    Some(match n {
        1 => "minecraft:black",
        2 => "minecraft:red",
        3 => "minecraft:siamese",
        4 => "minecraft:british",
        5 => "minecraft:calico",
        6 => "minecraft:persian",
        7 => "minecraft:ragdoll",
        8 => "minecraft:white",
        9 => "minecraft:jellie",
        10 => "minecraft:all_black",
        // The map's default return value.
        _ => "minecraft:tabby",
    })
}

/// `EntityVariantFix`: the numeric variant written to `variant`; the old field
/// stays (`updateAndRename` only sets the new key). An id the map has no entry
/// for gives Java's `null`, which `createString` cannot take: such a variant is
/// left as the number.
fn entity_variant(cx: &mut Cx, id: &str, field: &str, ids: fn(i32) -> Option<&'static str>) {
    cx.entity(id, |e| {
        let Some(value) = e.get(field).cloned() else {
            return;
        };
        let new = match as_number(&value).map(|n| ids(n.int_value())) {
            Some(Some(name)) => string(name),
            _ => value,
        };
        set(e, "variant", new);
    });
}

const CAT_TEXTURES: &[(&str, &str)] = &[
    ("textures/entity/cat/tabby.png", "minecraft:tabby"),
    ("textures/entity/cat/black.png", "minecraft:black"),
    ("textures/entity/cat/red.png", "minecraft:red"),
    ("textures/entity/cat/siamese.png", "minecraft:siamese"),
    (
        "textures/entity/cat/british_shorthair.png",
        "minecraft:british",
    ),
    ("textures/entity/cat/calico.png", "minecraft:calico"),
    ("textures/entity/cat/persian.png", "minecraft:persian"),
    ("textures/entity/cat/ragdoll.png", "minecraft:ragdoll"),
    ("textures/entity/cat/white.png", "minecraft:white"),
    ("textures/entity/cat/jellie.png", "minecraft:jellie"),
    ("textures/entity/cat/all_black.png", "minecraft:all_black"),
];

/// `CriteriaRenameFix`: the criteria keys of one advancement.
fn criteria_rename(cx: &mut Cx, advancement: &str, conversions: &dyn Fn(&str) -> Option<String>) {
    cx.each(Ty::Advancements, |v| {
        let Some(criteria) = v.get_mut(advancement).and_then(|a| a.get_mut("criteria")) else {
            return;
        };
        if let Tag::Compound(c) = criteria {
            let old = std::mem::take(c);
            for (k, value) in old {
                let name = k.to_string_lossy();
                let k = conversions(&name).map_or(k, |n| n.as_str().into());
                c.insert(k, value);
            }
        }
    });
}

// ------------------------------------------------------------------ 3094, 3097

/// The `tag` of every stack whose id passes `filter` (`ItemStackTagFix`).
fn item_tag(cx: &mut Cx, filter: impl Fn(&str) -> bool, mut f: impl FnMut(&mut Tag)) {
    cx.item(filter, |s| {
        if let Some(tag) = s.get_mut("tag") {
            f(tag);
        }
    });
}

const INSTRUMENTS: [&str; 8] = [
    "minecraft:ponder_goat_horn",
    "minecraft:sing_goat_horn",
    "minecraft:seek_goat_horn",
    "minecraft:feel_goat_horn",
    "minecraft:admire_goat_horn",
    "minecraft:call_goat_horn",
    "minecraft:yearn_goat_horn",
    "minecraft:dream_goat_horn",
];

/// `GoatHornIdFix`.
fn goat_horn_id(cx: &mut Cx) {
    item_tag(
        cx,
        |id| id == "minecraft:goat_horn",
        |tag| {
            let variant = super::dynamic::as_int(tag.get("SoundVariant"), 0);
            let sound = INSTRUMENTS[if (0..8).contains(&variant) {
                variant as usize
            } else {
                0
            }];
            remove(tag, "SoundVariant");
            set(tag, "instrument", string(sound));
        },
    );
}

/// `FilteredBooksFix`.
fn filtered_books(cx: &mut Cx) {
    item_tag(
        cx,
        |id| id == "minecraft:writable_book" || id == "minecraft:written_book",
        |tag| {
            remove(tag, "filtered_title");
            remove(tag, "filtered_pages");
        },
    );
}

/// `FilteredSignsFix` (a `NamedEntityWriteReadFix` over signs only).
fn filtered_signs(cx: &mut Cx) {
    let k = cx.k;
    cx.block_entity("minecraft:sign", |be| {
        for i in 1..=4 {
            remove(be, &format!("FilteredText{i}"));
        }
        reread_as(Ty::BlockEntity, k, be);
    });
}

/// `AbstractPoiSectionFix`: the records of every POI section kept by `keep`.
fn poi_records(cx: &mut Cx, keep: impl Fn(&Tag) -> bool) {
    cx.each(Ty::PoiChunk, |poi| {
        let Some(Tag::Compound(sections)) = poi.get_mut("Sections") else {
            return;
        };
        for section in sections.values_mut() {
            if let Some(records) = section.get("Records")
                && is_list(Some(records))
            {
                let kept: Vec<Tag> = stream(Some(records))
                    .into_iter()
                    .filter(|r| keep(r))
                    .collect();
                set(section, "Records", Tag::List(kept));
            }
        }
    });
}

// ------------------------------------------------------------------ 3209

/// `ItemStackSpawnEggFix` for `minecraft:pig_spawn_egg`: an egg whose entity has
/// an id becomes that entity's egg.
fn pig_spawn_egg(cx: &mut Cx) {
    cx.item(
        |id| id == "minecraft:pig_spawn_egg",
        |s| {
            let entity = s
                .get("tag")
                .and_then(|t| t.get("EntityTag"))
                .and_then(|e| as_str(e.get("id")));
            if let Some(entity) = entity {
                let egg = SPAWN_EGGS
                    .iter()
                    .find(|(e, _)| *e == namespaced(&entity))
                    .map_or("minecraft:pig_spawn_egg", |(_, egg)| egg);
                set(s, "id", string(egg));
            }
        },
    );
}

const SPAWN_EGGS: &[(&str, &str)] = &[
    ("minecraft:bat", "minecraft:bat_spawn_egg"),
    ("minecraft:blaze", "minecraft:blaze_spawn_egg"),
    ("minecraft:cave_spider", "minecraft:cave_spider_spawn_egg"),
    ("minecraft:chicken", "minecraft:chicken_spawn_egg"),
    ("minecraft:cow", "minecraft:cow_spawn_egg"),
    ("minecraft:creeper", "minecraft:creeper_spawn_egg"),
    ("minecraft:donkey", "minecraft:donkey_spawn_egg"),
    (
        "minecraft:elder_guardian",
        "minecraft:elder_guardian_spawn_egg",
    ),
    ("minecraft:ender_dragon", "minecraft:ender_dragon_spawn_egg"),
    ("minecraft:enderman", "minecraft:enderman_spawn_egg"),
    ("minecraft:endermite", "minecraft:endermite_spawn_egg"),
    (
        "minecraft:evocation_illager",
        "minecraft:evocation_illager_spawn_egg",
    ),
    ("minecraft:ghast", "minecraft:ghast_spawn_egg"),
    ("minecraft:guardian", "minecraft:guardian_spawn_egg"),
    ("minecraft:horse", "minecraft:horse_spawn_egg"),
    ("minecraft:husk", "minecraft:husk_spawn_egg"),
    ("minecraft:iron_golem", "minecraft:iron_golem_spawn_egg"),
    ("minecraft:llama", "minecraft:llama_spawn_egg"),
    ("minecraft:magma_cube", "minecraft:magma_cube_spawn_egg"),
    ("minecraft:mooshroom", "minecraft:mooshroom_spawn_egg"),
    ("minecraft:mule", "minecraft:mule_spawn_egg"),
    ("minecraft:ocelot", "minecraft:ocelot_spawn_egg"),
    ("minecraft:pufferfish", "minecraft:pufferfish_spawn_egg"),
    ("minecraft:parrot", "minecraft:parrot_spawn_egg"),
    ("minecraft:pig", "minecraft:pig_spawn_egg"),
    ("minecraft:polar_bear", "minecraft:polar_bear_spawn_egg"),
    ("minecraft:rabbit", "minecraft:rabbit_spawn_egg"),
    ("minecraft:sheep", "minecraft:sheep_spawn_egg"),
    ("minecraft:shulker", "minecraft:shulker_spawn_egg"),
    ("minecraft:silverfish", "minecraft:silverfish_spawn_egg"),
    ("minecraft:skeleton", "minecraft:skeleton_spawn_egg"),
    (
        "minecraft:skeleton_horse",
        "minecraft:skeleton_horse_spawn_egg",
    ),
    ("minecraft:slime", "minecraft:slime_spawn_egg"),
    ("minecraft:snow_golem", "minecraft:snow_golem_spawn_egg"),
    ("minecraft:spider", "minecraft:spider_spawn_egg"),
    ("minecraft:squid", "minecraft:squid_spawn_egg"),
    ("minecraft:stray", "minecraft:stray_spawn_egg"),
    ("minecraft:turtle", "minecraft:turtle_spawn_egg"),
    ("minecraft:vex", "minecraft:vex_spawn_egg"),
    ("minecraft:villager", "minecraft:villager_spawn_egg"),
    (
        "minecraft:vindication_illager",
        "minecraft:vindication_illager_spawn_egg",
    ),
    ("minecraft:witch", "minecraft:witch_spawn_egg"),
    ("minecraft:wither", "minecraft:wither_spawn_egg"),
    (
        "minecraft:wither_skeleton",
        "minecraft:wither_skeleton_spawn_egg",
    ),
    ("minecraft:wolf", "minecraft:wolf_spawn_egg"),
    ("minecraft:zombie", "minecraft:zombie_spawn_egg"),
    ("minecraft:zombie_horse", "minecraft:zombie_horse_spawn_egg"),
    (
        "minecraft:zombie_pigman",
        "minecraft:zombie_pigman_spawn_egg",
    ),
    (
        "minecraft:zombie_villager",
        "minecraft:zombie_villager_spawn_egg",
    ),
];

// ------------------------------------------------------------------ 3322

const POTION_ITEMS: [&str; 4] = [
    "minecraft:potion",
    "minecraft:splash_potion",
    "minecraft:lingering_potion",
    "minecraft:tipped_arrow",
];

/// `EffectDurationFix.fix`: every effect's factor data counts ticks active.
fn effect_list(list: Tag) -> Tag {
    if !is_list(Some(&list)) {
        return list;
    }
    Tag::List(
        stream(Some(&list))
            .into_iter()
            .map(|mut effect| {
                let duration = super::dynamic::as_int(effect.get("Duration"), -1);
                update(&mut effect, "FactorCalculationData", |mut data| {
                    let timestamp =
                        super::dynamic::as_int(data.get("effect_changed_timestamp"), -1);
                    remove(&mut data, "effect_changed_timestamp");
                    set(
                        &mut data,
                        "ticks_active",
                        Tag::Int(timestamp.wrapping_sub(duration)),
                    );
                    data
                });
                effect
            })
            .collect(),
    )
}

fn effect_duration(cx: &mut Cx) {
    let living = |e: &mut Tag| {
        for field in ["Effects", "ActiveEffects", "CustomPotionEffects"] {
            update(e, field, effect_list);
        }
    };
    cx.each(Ty::Entity, living);
    cx.each(Ty::Player, living);
    item_tag(
        cx,
        |id| POTION_ITEMS.contains(&id),
        |tag| update(tag, "CustomPotionEffects", effect_list),
    );
}

// ------------------------------------------------------------------ 3438, 3447

const POTTERY_SHARDS: &[(&str, &str)] = &[
    (
        "minecraft:pottery_shard_archer",
        "minecraft:archer_pottery_shard",
    ),
    (
        "minecraft:pottery_shard_prize",
        "minecraft:prize_pottery_shard",
    ),
    (
        "minecraft:pottery_shard_arms_up",
        "minecraft:arms_up_pottery_shard",
    ),
    (
        "minecraft:pottery_shard_skull",
        "minecraft:skull_pottery_shard",
    ),
];

const POTTERY_SHERDS: &[(&str, &str)] = &[
    (
        "minecraft:angler_pottery_shard",
        "minecraft:angler_pottery_sherd",
    ),
    (
        "minecraft:archer_pottery_shard",
        "minecraft:archer_pottery_sherd",
    ),
    (
        "minecraft:arms_up_pottery_shard",
        "minecraft:arms_up_pottery_sherd",
    ),
    (
        "minecraft:blade_pottery_shard",
        "minecraft:blade_pottery_sherd",
    ),
    (
        "minecraft:brewer_pottery_shard",
        "minecraft:brewer_pottery_sherd",
    ),
    (
        "minecraft:burn_pottery_shard",
        "minecraft:burn_pottery_sherd",
    ),
    (
        "minecraft:danger_pottery_shard",
        "minecraft:danger_pottery_sherd",
    ),
    (
        "minecraft:explorer_pottery_shard",
        "minecraft:explorer_pottery_sherd",
    ),
    (
        "minecraft:friend_pottery_shard",
        "minecraft:friend_pottery_sherd",
    ),
    (
        "minecraft:heart_pottery_shard",
        "minecraft:heart_pottery_sherd",
    ),
    (
        "minecraft:heartbreak_pottery_shard",
        "minecraft:heartbreak_pottery_sherd",
    ),
    (
        "minecraft:howl_pottery_shard",
        "minecraft:howl_pottery_sherd",
    ),
    (
        "minecraft:miner_pottery_shard",
        "minecraft:miner_pottery_sherd",
    ),
    (
        "minecraft:mourner_pottery_shard",
        "minecraft:mourner_pottery_sherd",
    ),
    (
        "minecraft:plenty_pottery_shard",
        "minecraft:plenty_pottery_sherd",
    ),
    (
        "minecraft:prize_pottery_shard",
        "minecraft:prize_pottery_sherd",
    ),
    (
        "minecraft:sheaf_pottery_shard",
        "minecraft:sheaf_pottery_sherd",
    ),
    (
        "minecraft:shelter_pottery_shard",
        "minecraft:shelter_pottery_sherd",
    ),
    (
        "minecraft:skull_pottery_shard",
        "minecraft:skull_pottery_sherd",
    ),
    (
        "minecraft:snort_pottery_shard",
        "minecraft:snort_pottery_sherd",
    ),
];

// ------------------------------------------------------------------ 3439

/// `LegacyComponentDataFixUtils.createEmptyComponent`.
fn empty_component() -> Tag {
    string("{\"text\":\"\"}")
}

/// `BlockEntitySignDoubleSidedEditableTextFix` (a `NamedEntityWriteReadFix`).
fn sign_double_sided(cx: &mut Cx, id: &str) {
    let k = cx.k;
    cx.block_entity(id, |be| {
        let lines: Vec<Tag> = (1..=4)
            .map(|i| {
                be.get(&format!("Text{i}"))
                    .cloned()
                    .unwrap_or_else(empty_component)
            })
            .collect();
        let filtered: Vec<Option<Tag>> = (1..=4)
            .map(|i| be.get(&format!("FilteredText{i}")).cloned())
            .collect();
        let mut front = Compound::new();
        front.insert("messages".into(), Tag::List(lines.clone()));
        front.insert(
            "color".into(),
            be.get("Color").cloned().unwrap_or_else(|| string("black")),
        );
        front.insert(
            "has_glowing_text".into(),
            be.get("GlowingText").cloned().unwrap_or(Tag::Byte(0)),
        );
        if filtered.iter().any(Option::is_some) {
            let messages = filtered
                .into_iter()
                .zip(&lines)
                .map(|(f, line)| f.unwrap_or_else(|| line.clone()))
                .collect();
            front.insert("filtered_messages".into(), Tag::List(messages));
        }
        set(be, "front_text", Tag::Compound(front));
        let back = Tag::compound([
            ("messages", Tag::List(vec![empty_component(); 4])),
            ("color", string("black")),
            ("has_glowing_text", Tag::Byte(0)),
        ]);
        set(be, "back_text", back);
        set(be, "is_waxed", Tag::Byte(0));
        set(be, "_filtered_correct", Tag::Byte(1));
        for field in super::fixes_a::SIGN_FIELDS_TO_DROP {
            remove(be, field);
        }
        reread_as(Ty::BlockEntity, k, be);
    });
}

// ------------------------------------------------------------------ 3440

/// `NamespacedTypeRenameFix` over `MULTI_NOISE_BIOME_SOURCE_PARAMETER_LIST`: the
/// preset of a multi-noise biome source in the world generation settings.
fn multi_noise_preset(cx: &mut Cx) {
    cx.each(Ty::WorldGenSettings, |v| {
        let Some(Tag::Compound(dims)) = v.get_mut("dimensions") else {
            return;
        };
        for dim in dims.values_mut() {
            let Some(g) = dim.get_mut("generator") else {
                continue;
            };
            if as_str(g.get("type")).as_deref() != Some("minecraft:noise") {
                continue;
            }
            let Some(source) = g.get_mut("biome_source") else {
                continue;
            };
            if as_str(source.get("type")).as_deref() != Some("minecraft:multi_noise") {
                continue;
            }
            if let Some(preset) = as_str(source.get("preset"))
                && namespaced(&preset) == "minecraft:overworld_update_1_20"
            {
                set(source, "preset", string("minecraft:overworld"));
            }
        }
    });
}

// ------------------------------------------------------------------ 3450, 3459

/// `RemapChunkStatusFix`: statuses namespaced, then mapped.
fn remap_chunk_status(cx: &mut Cx) {
    let fix = |v: Tag| match as_str(Some(&v)) {
        Some(s) => {
            let s = namespaced(&s);
            string(match s.as_str() {
                "minecraft:liquid_carvers" => "minecraft:carvers",
                "minecraft:heightmaps" => "minecraft:spawn",
                other => other,
            })
        }
        None => v,
    };
    cx.each(Ty::Chunk, |c| {
        update(c, "Status", fix);
        if let Some(retrogen) = c.get_mut("below_zero_retrogen") {
            update(retrogen, "target_status", fix);
        }
    });
}

/// `LegacyDragonFightFix`.
fn legacy_dragon_fight(cx: &mut Cx) {
    cx.each(Ty::Level, |level| {
        if level.get("DragonFight").is_some() {
            return;
        }
        let legacy = level
            .get("DimensionData")
            .and_then(|d| d.get("1"))
            .and_then(|d| d.get("DragonFight"))
            .cloned()
            .unwrap_or_else(super::dynamic::empty_map);
        let mut fight = legacy;
        update(&mut fight, "ExitPortalLocation", fix_block_pos);
        set(level, "DragonFight", fight);
    });
}
