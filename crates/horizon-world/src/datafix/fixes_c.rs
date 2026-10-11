//! Fixes of 1.21.6 to 26.3 (keys 4424 to 5016).
use super::dynamic::{
    as_bool, as_double, as_float, as_int, as_long, as_str, as_string, boolean, empty_map, int_list,
    namespaced, remove, rename, rename_and_fix, set, stream, string, update,
};
use super::fixes_a::{block_rename, component_fix};
use super::fixes_b::{advancements_rename, attributes_rename, feature_flag_remove};
use super::schema::{Key, Ty, key};
use super::{Cx, FixFn, id_is, renamer};
use crate::nbt::Tag;

pub(super) fn register(out: &mut Vec<(Key, FixFn)>) {
    // NamedEntityConvertUncheckedFix: the typed Particle is written as
    // custom_particle, the raw Particle comes back from the remainder.
    out.push((key(4420, 0), |cx| {
        cx.entity("minecraft:area_effect_cloud", |e| {
            if let Some(p) = e.get("Particle").cloned() {
                set(e, "custom_particle", p);
            }
            super::drop_typed(e, "Particle");
        });
    }));
    out.push((key(4424, 0), |cx| {
        feature_flag_remove(cx, &["minecraft:locator_bar"])
    }));
    out.push((key(4424, 0), |cx| {
        cx.each(Ty::Player, add_locator_bar_style)
    }));
    out.push((key(4424, 0), |cx| {
        cx.each(Ty::Entity, add_locator_bar_style)
    }));
    out.push((key(4535, 0), |cx| {
        cx.entity("minecraft:copper_golem", |e| {
            update(e, "weather_state", |v| {
                string(match as_int(Some(&v), 0) {
                    1 => "exposed",
                    2 => "weathered",
                    3 => "oxidized",
                    _ => "unaffected",
                })
            });
        });
    }));
    out.push((key(4537, 0), |cx| {
        cx.each(Ty::Chunk, |c| {
            remove(c, "isLightOn");
            if let Some(Tag::List(sections)) = c.get_mut("sections") {
                for s in sections {
                    remove(s, "BlockLight");
                    remove(s, "SkyLight");
                }
            }
        });
    }));
    out.push((key(4541, 0), |cx| {
        block_rename(cx, &[("minecraft:chain", "minecraft:iron_chain")])
    }));
    out.push((key(4541, 0), |cx| {
        cx.rename(
            Ty::ItemName,
            renamer(&[("minecraft:chain", "minecraft:iron_chain")]),
        )
    }));
    out.push((key(4544, 0), legacy_world_border));
    out.push((key(4548, 0), world_spawn_data));
    out.push((key(4548, 0), |cx| {
        cx.each(Ty::Player, |p| {
            update(p, "respawn", |mut r| {
                let dimension = as_string(r.get("dimension"), "minecraft:overworld");
                let yaw = as_float(r.get("angle"), 0.0);
                set(&mut r, "dimension", string(&dimension));
                set(&mut r, "yaw", Tag::Float(yaw.to_bits()));
                set(&mut r, "pitch", Tag::Float(0f32.to_bits()));
                remove(&mut r, "angle");
                r
            });
        });
    }));
    out.push((key(4649, 0), |cx| {
        component_fix(
            cx,
            "minecraft:consumable",
            "minecraft:consumable",
            |mut c| {
                if as_str(c.get("animation")).as_deref() == Some("spear") {
                    set(&mut c, "animation", string("trident"));
                }
                Some(c)
            },
        );
    }));
    out.push((key(4657, 0), |cx| {
        cx.each_out(Ty::SavedWorldBorder, |v| {
            if let Some(d) = v.get_mut("data") {
                update(d, "warning_time", |t| {
                    Tag::Int(as_int(Some(&t), 15).wrapping_mul(20))
                });
            }
        });
    }));
    out.push((key(4658, 0), game_rule_registry));
    out.push((key(4763, 0), |cx| {
        cx.entity("minecraft:villager", |e| {
            set(e, "VillagerDataFinalized", boolean(true))
        });
    }));
    out.push((key(4763, 0), |cx| {
        cx.entity("minecraft:zombie_villager", |e| {
            set(e, "VillagerDataFinalized", boolean(true))
        });
    }));
    out.push((key(4766, 0), day_time_to_clock));
    out.push((key(4769, 0), |cx| {
        cx.rename(Ty::Recipe, renamer(STONECUTTER_RECIPES))
    }));
    out.push((key(4769, 0), |cx| {
        advancements_rename(cx, &|id| {
            let id = namespaced(id);
            STONECUTTER_ADVANCEMENTS
                .iter()
                .find(|(from, _)| *from == id)
                .map(|(_, to)| (*to).to_owned())
        });
    }));
    out.push((key(4771, 0), level_dat_difficulty));
    out.push((key(4771, 0), level_dat_to_saved_data_preparation));
    out.push((key(4885, 0), remove_bed_block_entities));
    out.push((key(4888, 0), |cx| {
        attributes_rename(cx, false, &|id| {
            (id == "minecraft:nameplate_distance").then(|| "minecraft:name_tag_distance".into())
        });
    }));
    out.push((key(4996, 0), |cx| {
        cx.each_out(Ty::DataComponents, |c| {
            update(c, "minecraft:pot_decorations", unpack_pot_decorations)
        });
    }));
    out.push((key(4996, 1), |cx| {
        cx.block_entity("minecraft:decorated_pot", |be| {
            let sherds = be.get("sherds").cloned().unwrap_or(Tag::List(Vec::new()));
            set(be, "sherds", unpack_pot_decorations(sherds));
        });
    }));
    out.push((key(4997, 0), |cx| cx.each_out(Ty::Chunk, blending_data)));
    out.push((key(5002, 0), |cx| {
        cx.block_entity("minecraft:sign", |be| {
            set(be, "allow_op_features", boolean(true))
        });
    }));
    out.push((key(5002, 0), |cx| {
        cx.block_entity("minecraft:hanging_sign", |be| {
            set(be, "allow_op_features", boolean(true))
        });
    }));
    out.push((key(5006, 0), |cx| {
        cx.each(Ty::BlockState, |s| {
            rename(s, "Name", "id");
            rename(s, "Properties", "properties");
        });
    }));
    out.push((key(5007, 0), |cx| {
        cx.each(Ty::DataComponents, |c| {
            if let Some(swing) = remove(c, "minecraft:swing_animation") {
                set(c, "minecraft:attack_animation", swing.clone());
                set(c, "minecraft:interact_animation", swing);
            }
        });
    }));
    out.push((key(5008, 0), explorer_map_item));
    out.push((key(5008, 0), |cx| {
        component_fix(cx, "minecraft:map_color", "minecraft:map_color", |_| None)
    }));
    out.push((key(5009, 0), |cx| {
        cx.each(Ty::Chunk, |c| {
            if let Some(b) = c.get_mut("blending_data") {
                update(b, "heights", |h| {
                    Tag::List(
                        stream(Some(&h))
                            .iter()
                            .map(|v| {
                                let d = as_double(Some(v), f64::MAX);
                                let f = if d == f64::MAX { f32::MAX } else { d as f32 };
                                Tag::Float(f.to_bits())
                            })
                            .collect(),
                    )
                });
            }
        });
    }));
    out.push((key(5012, 0), |cx| {
        cx.rename(
            Ty::ItemName,
            renamer(&[
                (
                    "minecraft:ocean_explorer_map",
                    "minecraft:ocean_monument_map",
                ),
                ("minecraft:swamp_explorer_map", "minecraft:swamp_hut_map"),
                (
                    "minecraft:trial_explorer_map",
                    "minecraft:buried_trial_chambers_map",
                ),
                (
                    "minecraft:woodland_explorer_map",
                    "minecraft:woodland_mansion_map",
                ),
                (
                    "minecraft:jungle_explorer_map",
                    "minecraft:jungle_pyramid_map",
                ),
            ]),
        );
    }));
    out.push((key(5013, 0), merge_terrain_chunk_status));
    out.push((key(5014, 0), |cx| {
        cx.each(Ty::DataComponents, |c| {
            remove(c, "minecraft:block_transformer");
        });
    }));
    out.push((key(5016, 0), |cx| {
        cx.item(
            |id| id == "minecraft:map",
            |s| {
                if let Some(c) = s.get_mut("components")
                    && let Some(name) = c.get_mut("minecraft:item_name")
                    && plain_translation_key(name).as_deref() == Some("filled_map.buried_treasure")
                {
                    set(
                        name,
                        "translate",
                        string("item.minecraft.buried_treasure_map"),
                    );
                }
            },
        );
    }));
}

// ------------------------------------------------------------------ 4424

/// `AddFieldFix` with the path `locator_bar_icon`: as vanilla does it, the
/// value is replaced by its `locator_bar_icon` with a style added.
fn add_locator_bar_style(v: &mut Tag) {
    if let Some(mut icon) = v.get("locator_bar_icon").cloned() {
        set(&mut icon, "style", string("minecraft:default"));
        *v = icon;
    }
}

// ------------------------------------------------------------------ 4544, 4548, 4658, 4766

fn legacy_world_border(cx: &mut Cx) {
    cx.each(Ty::Level, |l| {
        let d = |l: &Tag, k: &str, def: f64| Tag::Double(as_double(l.get(k), def).to_bits());
        let border = Tag::compound([
            ("center_x", d(l, "BorderCenterX", 0.0)),
            ("center_z", d(l, "BorderCenterZ", 0.0)),
            ("size", d(l, "BorderSize", f64::from(5.999997E7f32))),
            (
                "lerp_time",
                Tag::Long(as_long(l.get("BorderSizeLerpTime"), 0)),
            ),
            ("lerp_target", d(l, "BorderSizeLerpTarget", 0.0)),
            ("safe_zone", d(l, "BorderSafeZone", 5.0)),
            ("damage_per_block", d(l, "BorderDamagePerBlock", 0.2)),
            (
                "warning_blocks",
                Tag::Int(as_int(l.get("BorderWarningBlocks"), 5)),
            ),
            (
                "warning_time",
                Tag::Int(as_int(l.get("BorderWarningTime"), 15)),
            ),
        ]);
        for k in [
            "BorderCenterX",
            "BorderCenterZ",
            "BorderSize",
            "BorderSizeLerpTime",
            "BorderSizeLerpTarget",
            "BorderSafeZone",
            "BorderDamagePerBlock",
            "BorderWarningBlocks",
            "BorderWarningTime",
        ] {
            remove(l, k);
        }
        set(l, "world_border", border);
    });
}

fn world_spawn_data(cx: &mut Cx) {
    cx.each(Ty::Level, |l| {
        let pos = [
            as_int(l.get("SpawnX"), 0),
            as_int(l.get("SpawnY"), 0),
            as_int(l.get("SpawnZ"), 0),
        ];
        let angle = as_float(l.get("SpawnAngle"), 0.0);
        for k in ["SpawnX", "SpawnY", "SpawnZ", "SpawnAngle"] {
            remove(l, k);
        }
        set(
            l,
            "spawn",
            Tag::compound([
                ("dimension", string("minecraft:overworld")),
                ("pos", int_list(&pos)),
                ("yaw", Tag::Float(angle.to_bits())),
                ("pitch", Tag::Float(0f32.to_bits())),
            ]),
        );
    });
}

const GAME_RULES: &[(&str, &str, Conv)] = &[
    (
        "allowEnteringNetherUsingPortals",
        "minecraft:allow_entering_nether_using_portals",
        Conv::Bool,
    ),
    (
        "announceAdvancements",
        "minecraft:show_advancement_messages",
        Conv::Bool,
    ),
    (
        "blockExplosionDropDecay",
        "minecraft:block_explosion_drop_decay",
        Conv::Bool,
    ),
    (
        "commandBlockOutput",
        "minecraft:command_block_output",
        Conv::Bool,
    ),
    (
        "enableCommandBlocks",
        "minecraft:command_blocks_work",
        Conv::Bool,
    ),
    (
        "commandBlocksEnabled",
        "minecraft:command_blocks_work",
        Conv::Bool,
    ),
    (
        "commandModificationBlockLimit",
        "minecraft:max_block_modifications",
        Conv::Int(1, i32::MAX),
    ),
    (
        "disableElytraMovementCheck",
        "minecraft:elytra_movement_check",
        Conv::NotBool,
    ),
    (
        "disablePlayerMovementCheck",
        "minecraft:player_movement_check",
        Conv::NotBool,
    ),
    ("disableRaids", "minecraft:raids", Conv::NotBool),
    ("doDaylightCycle", "minecraft:advance_time", Conv::Bool),
    ("doEntityDrops", "minecraft:entity_drops", Conv::Bool),
    (
        "doImmediateRespawn",
        "minecraft:immediate_respawn",
        Conv::Bool,
    ),
    ("doInsomnia", "minecraft:spawn_phantoms", Conv::Bool),
    (
        "doLimitedCrafting",
        "minecraft:limited_crafting",
        Conv::Bool,
    ),
    ("doMobLoot", "minecraft:mob_drops", Conv::Bool),
    ("doMobSpawning", "minecraft:spawn_mobs", Conv::Bool),
    ("doPatrolSpawning", "minecraft:spawn_patrols", Conv::Bool),
    ("doTileDrops", "minecraft:block_drops", Conv::Bool),
    (
        "doTraderSpawning",
        "minecraft:spawn_wandering_traders",
        Conv::Bool,
    ),
    ("doVinesSpread", "minecraft:spread_vines", Conv::Bool),
    ("doWardenSpawning", "minecraft:spawn_wardens", Conv::Bool),
    ("doWeatherCycle", "minecraft:advance_weather", Conv::Bool),
    ("drowningDamage", "minecraft:drowning_damage", Conv::Bool),
    (
        "enderPearlsVanishOnDeath",
        "minecraft:ender_pearls_vanish_on_death",
        Conv::Bool,
    ),
    ("fallDamage", "minecraft:fall_damage", Conv::Bool),
    ("fireDamage", "minecraft:fire_damage", Conv::Bool),
    (
        "forgiveDeadPlayers",
        "minecraft:forgive_dead_players",
        Conv::Bool,
    ),
    ("freezeDamage", "minecraft:freeze_damage", Conv::Bool),
    (
        "globalSoundEvents",
        "minecraft:global_sound_events",
        Conv::Bool,
    ),
    ("keepInventory", "minecraft:keep_inventory", Conv::Bool),
    (
        "lavaSourceConversion",
        "minecraft:lava_source_conversion",
        Conv::Bool,
    ),
    ("locatorBar", "minecraft:locator_bar", Conv::Bool),
    (
        "logAdminCommands",
        "minecraft:log_admin_commands",
        Conv::Bool,
    ),
    (
        "maxCommandChainLength",
        "minecraft:max_command_sequence_length",
        Conv::Int(0, i32::MAX),
    ),
    (
        "maxCommandForkCount",
        "minecraft:max_command_forks",
        Conv::Int(0, i32::MAX),
    ),
    (
        "maxEntityCramming",
        "minecraft:max_entity_cramming",
        Conv::Int(0, i32::MAX),
    ),
    (
        "minecartMaxSpeed",
        "minecraft:max_minecart_speed",
        Conv::Int(i32::MIN, i32::MAX),
    ),
    (
        "mobExplosionDropDecay",
        "minecraft:mob_explosion_drop_decay",
        Conv::Bool,
    ),
    ("mobGriefing", "minecraft:mob_griefing", Conv::Bool),
    (
        "naturalRegeneration",
        "minecraft:natural_health_regeneration",
        Conv::Bool,
    ),
    (
        "playersNetherPortalCreativeDelay",
        "minecraft:players_nether_portal_creative_delay",
        Conv::Int(0, i32::MAX),
    ),
    (
        "playersNetherPortalDefaultDelay",
        "minecraft:players_nether_portal_default_delay",
        Conv::Int(0, i32::MAX),
    ),
    (
        "playersSleepingPercentage",
        "minecraft:players_sleeping_percentage",
        Conv::Int(0, i32::MAX),
    ),
    (
        "projectilesCanBreakBlocks",
        "minecraft:projectiles_can_break_blocks",
        Conv::Bool,
    ),
    ("pvp", "minecraft:pvp", Conv::Bool),
    (
        "randomTickSpeed",
        "minecraft:random_tick_speed",
        Conv::Int(0, i32::MAX),
    ),
    (
        "reducedDebugInfo",
        "minecraft:reduced_debug_info",
        Conv::Bool,
    ),
    (
        "sendCommandFeedback",
        "minecraft:send_command_feedback",
        Conv::Bool,
    ),
    (
        "showDeathMessages",
        "minecraft:show_death_messages",
        Conv::Bool,
    ),
    (
        "snowAccumulationHeight",
        "minecraft:max_snow_accumulation_height",
        Conv::Int(0, 8),
    ),
    ("spawnMonsters", "minecraft:spawn_monsters", Conv::Bool),
    (
        "spawnRadius",
        "minecraft:respawn_radius",
        Conv::Int(i32::MIN, i32::MAX),
    ),
    (
        "spawnerBlocksEnabled",
        "minecraft:spawner_blocks_work",
        Conv::Bool,
    ),
    (
        "spectatorsGenerateChunks",
        "minecraft:spectators_generate_chunks",
        Conv::Bool,
    ),
    ("tntExplodes", "minecraft:tnt_explodes", Conv::Bool),
    (
        "tntExplosionDropDecay",
        "minecraft:tnt_explosion_drop_decay",
        Conv::Bool,
    ),
    ("universalAnger", "minecraft:universal_anger", Conv::Bool),
    (
        "waterSourceConversion",
        "minecraft:water_source_conversion",
        Conv::Bool,
    ),
];

#[derive(Clone, Copy)]
enum Conv {
    Bool,
    NotBool,
    Int(i32, i32),
}

/// `Boolean.parseBoolean`.
fn parse_bool(s: &str) -> bool {
    s.eq_ignore_ascii_case("true")
}

fn convert(v: Tag, conv: Conv) -> Tag {
    let s = as_string(Some(&v), "");
    match conv {
        Conv::Bool => boolean(parse_bool(&s)),
        Conv::NotBool => boolean(!parse_bool(&s)),
        Conv::Int(min, max) => match parse_int(&s) {
            Some(i) => Tag::Int(i.clamp(min, max)),
            None => v,
        },
    }
}

/// `Integer.parseInt`.
fn parse_int(s: &str) -> Option<i32> {
    let digits = s.strip_prefix(['+', '-']).unwrap_or(s);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

fn game_rule_registry(cx: &mut Cx) {
    cx.each(Ty::Level, |l| {
        rename_and_fix(l, "GameRules", "game_rules", |mut rules| {
            let fire_tick = parse_bool(&as_string(rules.get("doFireTick"), "true"));
            let away = parse_bool(&as_string(
                rules.get("allowFireTicksAwayFromPlayer"),
                "false",
            ));
            let radius = if !fire_tick {
                0
            } else if !away {
                128
            } else {
                -1
            };
            if radius != 128 {
                set(
                    &mut rules,
                    "minecraft:fire_spread_radius_around_player",
                    Tag::Int(radius),
                );
            }
            for k in [
                "spawnChunkRadius",
                "entitiesWithPassengersCanUsePortals",
                "gameLoopFunction",
                "doFireTick",
                "allowFireTicksAwayFromPlayer",
            ] {
                remove(&mut rules, k);
            }
            for (old, new, conv) in GAME_RULES {
                rename_and_fix(&mut rules, old, new, |v| Some(convert(v, *conv)));
            }
            Some(rules)
        });
    });
}

fn day_time_to_clock(cx: &mut Cx) {
    cx.each(Ty::Level, |l| {
        let game_time = as_long(l.get("Time"), 0);
        let day_time = as_long(l.get("DayTime"), game_time);
        remove(l, "DayTime");
        let clock = Tag::compound([
            ("total_ticks", Tag::Long(day_time)),
            ("paused", boolean(false)),
        ]);
        set(
            l,
            "world_clocks",
            Tag::compound([("minecraft:overworld", clock)]),
        );
    });
}

// ------------------------------------------------------------------ 4769, 4771

const STONECUTTER_RECIPES: &[(&str, &str)] = &[
    (
        "minecraft:chiseled_stone_bricks_stone_from_stonecutting",
        "minecraft:chiseled_stone_bricks_from_stone_stonecutting",
    ),
    (
        "minecraft:end_stone_brick_slab_from_end_stone_brick_stonecutting",
        "minecraft:end_stone_brick_slab_from_end_stone_bricks_stonecutting",
    ),
    (
        "minecraft:end_stone_brick_stairs_from_end_stone_brick_stonecutting",
        "minecraft:end_stone_brick_stairs_from_end_stone_bricks_stonecutting",
    ),
    (
        "minecraft:end_stone_brick_wall_from_end_stone_brick_stonecutting",
        "minecraft:end_stone_brick_wall_from_end_stone_bricks_stonecutting",
    ),
    (
        "minecraft:mossy_stone_brick_slab_from_mossy_stone_brick_stonecutting",
        "minecraft:mossy_stone_brick_slab_from_mossy_stone_bricks_stonecutting",
    ),
    (
        "minecraft:mossy_stone_brick_stairs_from_mossy_stone_brick_stonecutting",
        "minecraft:mossy_stone_brick_stairs_from_mossy_stone_bricks_stonecutting",
    ),
    (
        "minecraft:mossy_stone_brick_wall_from_mossy_stone_brick_stonecutting",
        "minecraft:mossy_stone_brick_wall_from_mossy_stone_bricks_stonecutting",
    ),
    (
        "minecraft:prismarine_brick_slab_from_prismarine_stonecutting",
        "minecraft:prismarine_brick_slab_from_prismarine_bricks_stonecutting",
    ),
    (
        "minecraft:prismarine_brick_stairs_from_prismarine_stonecutting",
        "minecraft:prismarine_brick_stairs_from_prismarine_bricks_stonecutting",
    ),
    (
        "minecraft:quartz_slab_from_stonecutting",
        "minecraft:quartz_slab_from_quartz_block_stonecutting",
    ),
    (
        "minecraft:stone_brick_walls_from_stone_stonecutting",
        "minecraft:stone_brick_wall_from_stone_stonecutting",
    ),
];
const STONECUTTER_ADVANCEMENTS: &[(&str, &str)] = &[
    (
        "minecraft:recipes/building_blocks/chiseled_stone_bricks_stone_from_stonecutting",
        "minecraft:recipes/building_blocks/chiseled_stone_bricks_from_stone_stonecutting",
    ),
    (
        "minecraft:recipes/building_blocks/end_stone_brick_slab_from_end_stone_brick_stonecutting",
        "minecraft:recipes/building_blocks/end_stone_brick_slab_from_end_stone_bricks_stonecutting",
    ),
    (
        "minecraft:recipes/building_blocks/end_stone_brick_stairs_from_end_stone_brick_stonecutting",
        "minecraft:recipes/building_blocks/end_stone_brick_stairs_from_end_stone_bricks_stonecutting",
    ),
    (
        "minecraft:recipes/decorations/end_stone_brick_wall_from_end_stone_brick_stonecutting",
        "minecraft:recipes/decorations/end_stone_brick_wall_from_end_stone_bricks_stonecutting",
    ),
    (
        "minecraft:recipes/building_blocks/mossy_stone_brick_slab_from_mossy_stone_brick_stonecutting",
        "minecraft:recipes/building_blocks/mossy_stone_brick_slab_from_mossy_stone_bricks_stonecutting",
    ),
    (
        "minecraft:recipes/building_blocks/mossy_stone_brick_stairs_from_mossy_stone_brick_stonecutting",
        "minecraft:recipes/building_blocks/mossy_stone_brick_stairs_from_mossy_stone_bricks_stonecutting",
    ),
    (
        "minecraft:recipes/building_blocks/mossy_stone_brick_wall_from_mossy_stone_brick_stonecutting",
        "minecraft:recipes/decorations/mossy_stone_brick_wall_from_mossy_stone_bricks_stonecutting",
    ),
    (
        "minecraft:recipes/building_blocks/prismarine_brick_slab_from_prismarine_stonecutting",
        "minecraft:recipes/building_blocks/prismarine_brick_slab_from_prismarine_bricks_stonecutting",
    ),
    (
        "minecraft:recipes/building_blocks/prismarine_brick_stairs_from_prismarine_stonecutting",
        "minecraft:recipes/building_blocks/prismarine_brick_stairs_from_prismarine_bricks_stonecutting",
    ),
    (
        "minecraft:recipes/building_blocks/quartz_slab_from_stonecutting",
        "minecraft:recipes/building_blocks/quartz_slab_from_quartz_block_stonecutting",
    ),
    (
        "minecraft:recipes/decorations/stone_brick_walls_from_stone_stonecutting",
        "minecraft:recipes/decorations/stone_brick_wall_from_stone_stonecutting",
    ),
];

fn level_dat_difficulty(cx: &mut Cx) {
    cx.each(Ty::LightweightLevel, |l| {
        let difficulty = match as_int(l.get("Difficulty"), 2) {
            0 => "peaceful",
            1 => "easy",
            3 => "hard",
            _ => "normal",
        };
        let settings = Tag::compound([
            ("difficulty", string(difficulty)),
            ("hardcore", boolean(as_bool(l.get("hardcore"), false))),
            ("locked", boolean(as_bool(l.get("DifficultyLocked"), false))),
        ]);
        set(l, "difficulty_settings", settings);
        remove(l, "Difficulty");
        remove(l, "hardcore");
        remove(l, "DifficultyLocked");
    });
}

fn level_dat_to_saved_data_preparation(cx: &mut Cx) {
    cx.each(Ty::Level, |l| {
        let source = l.clone();
        rename_and_fix(l, "DragonFight", "dragon_fight", |mut fight| {
            let respawning = as_bool(fight.get("IsRespawning"), false);
            for (a, b) in [
                ("NeedsStateScanning", "needs_state_scanning"),
                ("DragonKilled", "dragon_killed"),
                ("PreviouslyKilled", "previously_killed"),
                ("Dragon", "dragon_uuid"),
                ("ExitPortalLocation", "exit_portal_location"),
                ("Gateways", "gateways"),
            ] {
                rename(&mut fight, a, b);
            }
            if respawning {
                set(&mut fight, "respawn_stage", string("start"));
                set(&mut fight, "respawn_time", Tag::Int(0));
            }
            remove(&mut fight, "IsRespawning");
            Some(fight)
        });
        let trader = Tag::compound([
            (
                "spawn_delay",
                Tag::Int(as_int(source.get("WanderingTraderSpawnDelay"), 24000)),
            ),
            (
                "spawn_chance",
                Tag::Int(as_int(source.get("WanderingTraderSpawnChance"), 25)),
            ),
        ]);
        set(l, "wandering_trader_migration_data", trader);
        for k in [
            "WanderingTraderSpawnDelay",
            "WanderingTraderSpawnChance",
            "WanderingTraderId",
        ] {
            remove(l, k);
        }
        let weather = Tag::compound([
            (
                "clear_weather_time",
                Tag::Int(as_int(source.get("clearWeatherTime"), 0)),
            ),
            ("rain_time", Tag::Int(as_int(source.get("rainTime"), 0))),
            (
                "thunder_time",
                Tag::Int(as_int(source.get("thunderTime"), 0)),
            ),
            ("raining", boolean(as_bool(source.get("raining"), false))),
            (
                "thundering",
                boolean(as_bool(source.get("thundering"), false)),
            ),
        ]);
        for k in [
            "clearWeatherTime",
            "rainTime",
            "thunderTime",
            "raining",
            "thundering",
        ] {
            remove(l, k);
        }
        set(l, "weather_data", weather);
        // `asList` of something that is not a list is empty.
        let events: Vec<Tag> = stream(source.get("ScheduledEvents"))
            .into_iter()
            .map(|mut e| {
                rename(&mut e, "Name", "id");
                rename(&mut e, "TriggerTime", "trigger_time");
                rename_and_fix(&mut e, "Callback", "callback", |mut c| {
                    rename(&mut c, "Type", "type");
                    rename(&mut c, "Name", "id");
                    Some(c)
                });
                e
            })
            .collect();
        remove(l, "ScheduledEvents");
        set(
            l,
            "scheduled_events",
            Tag::compound([("events", Tag::List(events))]),
        );
        rename_and_fix(l, "WorldGenSettings", "world_gen_settings", |mut s| {
            rename(&mut s, "generate_features", "generate_structures");
            Some(s)
        });
    });
}

// ------------------------------------------------------------------ 4885, 4996, 4997

/// `RemoveBlockEntityTagFix` for beds: item block entity data, falling block
/// entity data and chunk block entities of beds go.
fn remove_bed_block_entities(cx: &mut Cx) {
    let is_bed = |v: &Tag| id_is(v, "minecraft:bed");
    cx.each(Ty::ItemStack, |s| {
        if let Some(c) = s.get_mut("components")
            && c.get("minecraft:block_entity_data").is_some_and(is_bed)
        {
            remove(c, "minecraft:block_entity_data");
        }
    });
    cx.entity("minecraft:falling_block", |e| {
        if e.get("TileEntityData").is_some_and(is_bed) {
            remove(e, "TileEntityData");
        }
    });
    cx.each(Ty::Chunk, |c| {
        if let Some(Tag::List(bes)) = c.get_mut("block_entities") {
            bes.retain(|be| !is_bed(be));
        }
    });
}

/// `PotDecorationsComponentUnflatteningFix.unpackList`.
fn unpack_pot_decorations(original: Tag) -> Tag {
    if !super::dynamic::is_list(Some(&original)) {
        return original;
    }
    let ids: Vec<Option<String>> = stream(Some(&original))
        .iter()
        .map(|t| as_str(Some(t)))
        .collect();
    let mut result = empty_map();
    for (i, side) in ["back", "left", "right", "front"].iter().enumerate() {
        let id = ids
            .get(i)
            .cloned()
            .flatten()
            .unwrap_or_else(|| "minecraft:brick".into());
        if id.is_empty() {
            return original;
        }
        set(&mut result, side, Tag::compound([("id", string(&id))]));
    }
    result
}

fn blending_data(c: &mut Tag) {
    remove(c, "blending_data");
    let overworld = c
        .get("__context")
        .and_then(|ctx| as_str(ctx.get("dimension")))
        .is_some_and(|d| d == "minecraft:overworld");
    let Some(status) = c.get("Status") else {
        return;
    };
    if !overworld {
        return;
    }
    const SKIP: [&str; 4] = [
        "minecraft:empty",
        "minecraft:structure_starts",
        "minecraft:structure_references",
        "minecraft:biomes",
    ];
    let status = namespaced(&as_string(Some(status), "empty"));
    let blending = |height: i32, min_y: i32| {
        Tag::compound([
            ("min_section", Tag::Int(min_y >> 4)),
            ("max_section", Tag::Int((min_y + height) >> 4)),
        ])
    };
    if !SKIP.contains(&status.as_str()) {
        set(c, "blending_data", blending(384, -64));
    } else if let Some(retrogen) = c.get("below_zero_retrogen") {
        let target = namespaced(&as_string(retrogen.get("target_status"), "empty"));
        if !SKIP.contains(&target.as_str()) {
            set(c, "blending_data", blending(256, 0));
        }
    }
}

// ------------------------------------------------------------------ 5008, 5013

/// `ExtraDataFixUtils.getPlainTranslationKey`: `{translate: key}` alone.
fn plain_translation_key(v: &Tag) -> Option<String> {
    match v {
        Tag::Compound(c) if c.len() == 1 => as_str(v.get("translate")),
        _ => None,
    }
}

const EXPLORER_MAPS: &[(&str, &str, &str)] = &[
    (
        "minecraft:mansion",
        "minecraft:woodland_explorer_map",
        "filled_map.mansion",
    ),
    (
        "minecraft:monument",
        "minecraft:ocean_explorer_map",
        "filled_map.monument",
    ),
    (
        "minecraft:trial_chambers",
        "minecraft:trial_explorer_map",
        "filled_map.trial_chambers",
    ),
    (
        "minecraft:jungle_temple",
        "minecraft:jungle_explorer_map",
        "filled_map.explorer_jungle",
    ),
    (
        "minecraft:swamp_hut",
        "minecraft:swamp_explorer_map",
        "filled_map.explorer_swamp",
    ),
    (
        "minecraft:village_desert",
        "minecraft:desert_village_map",
        "filled_map.village_desert",
    ),
    (
        "minecraft:village_plains",
        "minecraft:plains_village_map",
        "filled_map.village_plains",
    ),
    (
        "minecraft:village_savanna",
        "minecraft:savanna_village_map",
        "filled_map.village_savanna",
    ),
    (
        "minecraft:village_snowy",
        "minecraft:snowy_village_map",
        "filled_map.village_snowy",
    ),
    (
        "minecraft:village_taiga",
        "minecraft:taiga_village_map",
        "filled_map.village_taiga",
    ),
    (
        "minecraft:red_x",
        "minecraft:buried_treasure_map",
        "filled_map.buried_treasure",
    ),
];

fn explorer_map_item(cx: &mut Cx) {
    cx.item(
        |id| id == "minecraft:filled_map",
        |s| {
            let Some(decoration) = s
                .get("components")
                .and_then(|c| c.get("minecraft:map_decorations"))
                .and_then(|d| d.get("+"))
            else {
                return;
            };
            let ty = namespaced(&as_string(decoration.get("type"), ""));
            let Some((_, item, legacy)) = EXPLORER_MAPS.iter().find(|(t, _, _)| *t == ty) else {
                return;
            };
            set(s, "id", string(item));
            if let Some(c) = s.get_mut("components")
                && c.get("minecraft:item_name")
                    .and_then(plain_translation_key)
                    .as_deref()
                    == Some(*legacy)
            {
                remove(c, "minecraft:item_name");
            }
        },
    );
}

fn merge_terrain_chunk_status(cx: &mut Cx) {
    cx.each(Ty::Chunk, |c| {
        const INTERMEDIATE: [&str; 2] = ["minecraft:noise", "minecraft:surface"];
        let fix = |s: &str| -> String {
            if s == "minecraft:carvers" {
                "minecraft:terrain".into()
            } else if INTERMEDIATE.contains(&s) {
                "minecraft:biomes".into()
            } else {
                s.to_owned()
            }
        };
        let old_status = namespaced(&as_string(c.get("Status"), "minecraft:empty"));
        set(c, "Status", string(&fix(&old_status)));
        let mut old_target = None;
        if let Some(r) = c.get_mut("below_zero_retrogen") {
            let target = namespaced(&as_string(r.get("target_status"), "minecraft:empty"));
            set(r, "target_status", string(&fix(&target)));
            old_target = Some(target);
        }
        let status_intermediate = INTERMEDIATE.contains(&old_status.as_str());
        let target_intermediate = old_target
            .as_deref()
            .is_some_and(|t| INTERMEDIATE.contains(&t));
        if status_intermediate || target_intermediate {
            let only_below_zero = old_target.is_some() && !target_intermediate;
            remove(c, "Heightmaps");
            remove(c, "blending_data");
            if let Some(Tag::List(sections)) = c.get_mut("sections") {
                for s in sections {
                    let y = super::dynamic::as_number(s.get("Y").unwrap_or(&Tag::End))
                        .map_or(0, |n| n.byte_value());
                    if !(y >= 0 && only_below_zero) {
                        remove(s, "block_states");
                    }
                }
            }
        }
    });
}
