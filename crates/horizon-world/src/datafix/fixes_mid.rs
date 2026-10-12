//! Fixes of 1.15 and 1.16 (keys 2100 to 2571): what worlds saved by 1.14.4 to
//! 1.16.5 need before the fixes of [`super::fixes_cliffs`].
use super::dynamic::{
    as_bool, as_double, as_int, as_long, as_long_stream, as_number, as_str, ceillog2, namespaced,
    remove, set, stream, string,
};
use super::fixes_a::attributes_rename_legacy;
use super::fixes_b::advancements_rename;
use super::schema::{Key, Ty, key};
use super::{Cx, FixFn, id_is, renamer};
use crate::nbt::{Compound, JavaString, Tag};

pub(super) fn register(out: &mut Vec<(Key, FixFn)>) {
    out.push((key(2100, 0), |cx| {
        cx.rename(
            Ty::Recipe,
            renamer(&[("minecraft:sugar", "minecraft:sugar_from_sugar_cane")]),
        )
    }));
    out.push((key(2100, 0), |cx| {
        advancements_rename_table(
            cx,
            &[(
                "minecraft:recipes/misc/sugar",
                "minecraft:recipes/misc/sugar_from_sugar_cane",
            )],
        )
    }));
    out.push((key(2202, 0), chunk_biomes));
    out.push((key(2209, 0), |cx| {
        let t: &[(&str, &'static str)] = &[("minecraft:bee_hive", "minecraft:beehive")];
        cx.rename(Ty::ItemName, renamer(t));
        poi_type_rename(cx, t);
    }));
    out.push((key(2209, 0), |cx| {
        super::fixes_a::block_rename(cx, &[("minecraft:bee_hive", "minecraft:beehive")])
    }));
    out.push((key(2211, 0), structure_reference_count));
    out.push((key(2218, 0), force_poi_rebuild));
    out.push((key(2501, 0), furnace_recipes));
    out.push((key(2503, 0), wall_property));
    out.push((key(2503, 0), |cx| {
        advancements_rename_table(
            cx,
            &[(
                "minecraft:recipes/misc/composter",
                "minecraft:recipes/decorations/composter",
            )],
        )
    }));
    out.push((key(2505, 0), |cx| memory_expiry(cx, "minecraft:villager")));
    out.push((key(2508, 0), |cx| cx.rename(Ty::ItemName, renamer(FUNGUS))));
    out.push((key(2508, 0), |cx| super::fixes_a::block_rename(cx, FUNGUS)));
    out.push((key(2509, 0), |cx| {
        entity_rename(cx, "minecraft:zombie_pigman", "minecraft:zombified_piglin")
    }));
    out.push((key(2509, 0), |cx| {
        cx.rename(
            Ty::ItemName,
            renamer(&[(
                "minecraft:zombie_pigman_spawn_egg",
                "minecraft:zombified_piglin_spawn_egg",
            )]),
        )
    }));
    out.push((key(2511, 0), projectile_owner));
    out.push((key(2514, 0), entity_uuids));
    out.push((key(2514, 0), block_entity_uuids));
    out.push((key(2514, 0), player_uuids));
    out.push((key(2514, 0), level_uuids));
    out.push((key(2514, 0), raids_uuids));
    out.push((key(2514, 0), item_stack_uuids));
    out.push((key(2516, 0), |cx| gossip_uuids(cx, "minecraft:villager")));
    out.push((key(2516, 0), |cx| {
        gossip_uuids(cx, "minecraft:zombie_villager")
    }));
    out.push((key(2518, 0), jigsaw_properties));
    out.push((key(2518, 0), jigsaw_rotation));
    out.push((key(2523, 0), |cx| {
        attributes_rename_legacy(cx, &|name| {
            ATTRIBUTES
                .iter()
                .find(|(from, _)| *from == name)
                .map(|(_, to)| *to)
        })
    }));
    out.push((key(2527, 0), bit_storage_align));
    out.push((key(2528, 0), |cx| {
        cx.rename(
            Ty::ItemName,
            renamer(&[
                ("minecraft:soul_fire_torch", "minecraft:soul_torch"),
                ("minecraft:soul_fire_lantern", "minecraft:soul_lantern"),
            ]),
        )
    }));
    out.push((key(2528, 0), |cx| {
        super::fixes_a::block_rename(
            cx,
            &[
                ("minecraft:soul_fire_torch", "minecraft:soul_torch"),
                (
                    "minecraft:soul_fire_wall_torch",
                    "minecraft:soul_wall_torch",
                ),
                ("minecraft:soul_fire_lantern", "minecraft:soul_lantern"),
            ],
        )
    }));
    out.push((key(2529, 0), strider_gravity));
    out.push((key(2531, 0), redstone_wire));
    out.push((key(2533, 0), villager_follow_range));
    out.push((key(2535, 0), shulker_rotation));
}

const FUNGUS: &[(&str, &str)] = &[
    ("minecraft:warped_fungi", "minecraft:warped_fungus"),
    ("minecraft:crimson_fungi", "minecraft:crimson_fungus"),
];

/// `AdvancementsRenameFix` with a `createRenamer` table.
fn advancements_rename_table(cx: &mut Cx, table: &'static [(&str, &str)]) {
    advancements_rename(cx, &|id| {
        let id = namespaced(id);
        table
            .iter()
            .find(|(from, _)| *from == id)
            .map(|(_, to)| (*to).to_owned())
    });
}

/// `SimplestEntityRenameFix`: the id of the entities and the entity names.
fn entity_rename(cx: &mut Cx, from: &str, to: &'static str) {
    cx.each(Ty::Entity, |e| {
        if id_is(e, from) {
            set(e, "id", string(to));
        }
    });
    cx.rename(Ty::EntityName, |name| {
        (namespaced(name) == from).then_some(to)
    });
}

/// `PoiTypeRenameFix`.
fn poi_type_rename(cx: &mut Cx, table: &[(&str, &'static str)]) {
    let r = renamer(table);
    cx.each(Ty::PoiChunk, |poi| {
        let Some(Tag::Compound(sections)) = poi.get_mut("Sections") else {
            return;
        };
        for section in sections.values_mut() {
            let Some(Tag::List(records)) = section.get_mut("Records") else {
                continue;
            };
            for record in records {
                if let Some(t) = as_str(record.get("type"))
                    && let Some(new) = r(&t)
                {
                    set(record, "type", string(new));
                }
            }
        }
    });
}

/// `ChunkBiomeFix`: the 256 biomes of a column become 1024 (a 4x4x64 grid).
fn chunk_biomes(cx: &mut Cx) {
    cx.each(Ty::Chunk, |c| {
        let Some(level) = c.get_mut("Level") else {
            return;
        };
        let Some(old) = super::dynamic::as_int_stream(level.get("Biomes")) else {
            return;
        };
        if old.len() != 256 {
            return;
        }
        let mut new = vec![0i32; 1024];
        for z in 0..4 {
            for x in 0..4 {
                let (ox, oz) = ((x << 2) + 2, (z << 2) + 2);
                new[z << 2 | x] = old[oz << 4 | ox];
            }
        }
        for y in 1..64 {
            new.copy_within(0..16, y * 16);
        }
        set(level, "Biomes", Tag::IntArray(new));
    });
}

/// `StructureReferenceCountFix`.
fn structure_reference_count(cx: &mut Cx) {
    cx.each(Ty::StructureFeature, |s| {
        if let Some(r) = s.get("references") {
            let n = as_number(r)
                .map(|n| n.int_value())
                .filter(|n| *n > 0)
                .unwrap_or(1);
            set(s, "references", Tag::Int(n));
        }
    });
}

/// `ForcePoiRebuild`: every POI section loses its `Valid` flag.
fn force_poi_rebuild(cx: &mut Cx) {
    cx.each(Ty::PoiChunk, |poi| {
        if let Some(Tag::Compound(sections)) = poi.get_mut("Sections") {
            for section in sections.values_mut() {
                remove(section, "Valid");
            }
        }
    });
}

/// `FurnaceRecipeFix`: `RecipeLocation<i>`/`RecipeAmount<i>` pairs become the
/// `RecipesUsed` map.
fn furnace_recipes(cx: &mut Cx) {
    cx.each(Ty::BlockEntity, |be| {
        if ![
            "minecraft:furnace",
            "minecraft:blast_furnace",
            "minecraft:smoker",
        ]
        .iter()
        .any(|id| id_is(be, id))
        {
            return;
        }
        let size = as_int(be.get("RecipesUsedSize"), 0);
        remove(be, "RecipesUsedSize");
        let mut used = Compound::new();
        for i in 0..size {
            let location = remove(be, &format!("RecipeLocation{i}"));
            let amount = remove(be, &format!("RecipeAmount{i}"));
            let amount = as_int(amount.as_ref(), 0);
            if amount > 0
                && let Some(location) = location.as_ref().and_then(|l| as_str(Some(l)))
            {
                used.insert(
                    JavaString::from(namespaced(&location).as_str()),
                    Tag::Int(amount),
                );
            }
        }
        set(be, "RecipesUsed", Tag::Compound(used));
    });
}

const WALLS: [&str; 14] = [
    "minecraft:andesite_wall",
    "minecraft:brick_wall",
    "minecraft:cobblestone_wall",
    "minecraft:diorite_wall",
    "minecraft:end_stone_brick_wall",
    "minecraft:granite_wall",
    "minecraft:mossy_cobblestone_wall",
    "minecraft:mossy_stone_brick_wall",
    "minecraft:nether_brick_wall",
    "minecraft:prismarine_wall",
    "minecraft:red_nether_brick_wall",
    "minecraft:red_sandstone_wall",
    "minecraft:sandstone_wall",
    "minecraft:stone_brick_wall",
];

/// `WallPropertyFix`: `true`/`false` sides become `low`/`none`.
fn wall_property(cx: &mut Cx) {
    cx.each(Ty::BlockState, |state| {
        let Some(name) = as_str(state.get("Name")) else {
            return;
        };
        if !WALLS.contains(&name.as_str()) {
            return;
        }
        if let Some(props) = state.get_mut("Properties") {
            for side in ["east", "west", "north", "south"] {
                if let Some(v) = props.get(side)
                    && let Some(s) = as_str(Some(v))
                {
                    let new = if s == "true" { "low" } else { "none" };
                    set(props, side, string(new));
                }
            }
        }
    });
}

/// `MemoryExpiryDataFix`: every memory of a brain is wrapped as `{value: ...}`.
fn memory_expiry(cx: &mut Cx, entity: &str) {
    cx.entity(entity, |e| {
        let Some(brain) = e.get_mut("Brain") else {
            return;
        };
        if let Some(Tag::Compound(memories)) = brain.get_mut("memories") {
            for value in memories.values_mut() {
                let inner = std::mem::replace(value, Tag::End);
                *value = Tag::compound([("value", inner)]);
            }
        }
    });
}

/// `EntityProjectileOwnerFix`: the owner of a projectile as a UUID int array.
fn projectile_owner(cx: &mut Cx) {
    let uuid = |most: i64, least: i64| -> Option<Tag> {
        (most != 0 && least != 0).then(|| {
            Tag::IntArray(vec![
                (most >> 32) as i32,
                most as i32,
                (least >> 32) as i32,
                least as i32,
            ])
        })
    };
    cx.each(Ty::Entity, |e| {
        let Some(id) = as_str(e.get("id")).map(|i| namespaced(&i)) else {
            return;
        };
        match id.as_str() {
            "minecraft:egg"
            | "minecraft:ender_pearl"
            | "minecraft:experience_bottle"
            | "minecraft:snowball"
            | "minecraft:potion" => {
                let (m, l) = e
                    .get("owner")
                    .map_or((0, 0), |o| (as_long(o.get("M"), 0), as_long(o.get("L"), 0)));
                if let Some(u) = uuid(m, l) {
                    set(e, "OwnerUUID", u);
                }
                remove(e, "owner");
            }
            "minecraft:llama_spit" => {
                let (m, l) = e.get("Owner").map_or((0, 0), |o| {
                    (
                        as_long(o.get("OwnerUUIDMost"), 0),
                        as_long(o.get("OwnerUUIDLeast"), 0),
                    )
                });
                if let Some(u) = uuid(m, l) {
                    set(e, "OwnerUUID", u);
                }
                remove(e, "Owner");
            }
            "minecraft:arrow" | "minecraft:spectral_arrow" | "minecraft:trident" => {
                let m = as_long(e.get("OwnerUUIDMost"), 0);
                let l = as_long(e.get("OwnerUUIDLeast"), 0);
                if let Some(u) = uuid(m, l) {
                    set(e, "OwnerUUID", u);
                }
                remove(e, "OwnerUUIDMost");
                remove(e, "OwnerUUIDLeast");
            }
            _ => {}
        }
    });
}

// ------------------------------------------------------------------ 2514, 2516

fn uuid_tag(most: i64, least: i64) -> Tag {
    Tag::IntArray(vec![
        (most >> 32) as i32,
        most as i32,
        (least >> 32) as i32,
        least as i32,
    ])
}

/// `AbstractUUIDFix.createUUIDFromLongs`.
fn uuid_from_longs(tag: &Tag, most: &str, least: &str) -> Option<Tag> {
    let m = as_long(tag.get(most), 0);
    let l = as_long(tag.get(least), 0);
    (m != 0 && l != 0).then(|| uuid_tag(m, l))
}

/// `AbstractUUIDFix.replaceUUIDLeastMost`.
fn replace_least_most(tag: &mut Tag, old: &str, new: &str) {
    let (most, least) = (format!("{old}Most"), format!("{old}Least"));
    if let Some(uuid) = uuid_from_longs(tag, &most, &least) {
        remove(tag, &most);
        remove(tag, &least);
        set(tag, new, uuid);
    }
}

/// `AbstractUUIDFix.replaceUUIDMLTag`.
fn replace_ml(tag: &mut Tag, old: &str, new: &str) {
    if let Some(uuid) = tag.get(old).and_then(|t| uuid_from_longs(t, "M", "L")) {
        remove(tag, old);
        set(tag, new, uuid);
    }
}

/// `UUID.fromString`, which takes five hexadecimal groups of any length.
fn parse_uuid(s: &str) -> Option<(i64, i64)> {
    if s.len() > 36 {
        return None;
    }
    let mut parts = [0i64; 5];
    let mut count = 0;
    for part in s.split('-') {
        if count == 5 || part.is_empty() || !part.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        parts[count] = i64::from_str_radix(part, 16).ok()?;
        count += 1;
    }
    (count == 5).then(|| {
        let most = (parts[0] & 0xffff_ffff) << 32 | (parts[1] & 0xffff) << 16 | (parts[2] & 0xffff);
        let least = (parts[3] & 0xffff) << 48 | (parts[4] & 0xffff_ffff_ffff);
        (most, least)
    })
}

/// `AbstractUUIDFix.replaceUUIDString`.
fn replace_uuid_string(tag: &mut Tag, old: &str, new: &str) -> bool {
    let Some((m, l)) = as_str(tag.get(old)).as_deref().and_then(parse_uuid) else {
        return false;
    };
    remove(tag, old);
    set(tag, new, uuid_tag(m, l));
    true
}

/// A field that is present becomes the list of its elements mapped by `f`.
fn map_elements(tag: &mut Tag, field: &str, mut f: impl FnMut(Tag) -> Tag) {
    if let Some(v) = tag.get_mut(field) {
        *v = Tag::List(stream(Some(v)).into_iter().map(&mut f).collect());
    }
}

/// `EntityUUIDFix.updateLivingEntity`.
fn living_entity_uuids(tag: &mut Tag) {
    map_elements(tag, "Attributes", |mut attribute| {
        map_elements(&mut attribute, "Modifiers", |mut modifier| {
            replace_least_most(&mut modifier, "UUID", "UUID");
            modifier
        });
        attribute
    });
}

fn mob_uuids(tag: &mut Tag) {
    living_entity_uuids(tag);
    if let Some(leash) = tag.get_mut("Leash") {
        replace_least_most(leash, "UUID", "UUID");
    }
}

fn animal_uuids(tag: &mut Tag) {
    mob_uuids(tag);
    replace_least_most(tag, "LoveCause", "LoveCause");
}

const ABSTRACT_HORSES: &[&str] = [
    "minecraft:donkey",
    "minecraft:horse",
    "minecraft:llama",
    "minecraft:mule",
    "minecraft:skeleton_horse",
    "minecraft:trader_llama",
    "minecraft:zombie_horse",
];
const TAMEABLE_ANIMALS: &[&str] = ["minecraft:cat", "minecraft:parrot", "minecraft:wolf"];
const ANIMALS: &[&str] = [
    "minecraft:bee",
    "minecraft:chicken",
    "minecraft:cow",
    "minecraft:fox",
    "minecraft:mooshroom",
    "minecraft:ocelot",
    "minecraft:panda",
    "minecraft:pig",
    "minecraft:polar_bear",
    "minecraft:rabbit",
    "minecraft:sheep",
    "minecraft:turtle",
    "minecraft:hoglin",
];
const MOBS: &[&str] = [
    "minecraft:bat",
    "minecraft:blaze",
    "minecraft:cave_spider",
    "minecraft:cod",
    "minecraft:creeper",
    "minecraft:dolphin",
    "minecraft:drowned",
    "minecraft:elder_guardian",
    "minecraft:ender_dragon",
    "minecraft:enderman",
    "minecraft:endermite",
    "minecraft:evoker",
    "minecraft:ghast",
    "minecraft:giant",
    "minecraft:guardian",
    "minecraft:husk",
    "minecraft:illusioner",
    "minecraft:magma_cube",
    "minecraft:pufferfish",
    "minecraft:zombified_piglin",
    "minecraft:salmon",
    "minecraft:shulker",
    "minecraft:silverfish",
    "minecraft:skeleton",
    "minecraft:slime",
    "minecraft:snow_golem",
    "minecraft:spider",
    "minecraft:squid",
    "minecraft:stray",
    "minecraft:tropical_fish",
    "minecraft:vex",
    "minecraft:villager",
    "minecraft:iron_golem",
    "minecraft:vindicator",
    "minecraft:pillager",
    "minecraft:wandering_trader",
    "minecraft:witch",
    "minecraft:wither",
    "minecraft:wither_skeleton",
    "minecraft:zombie",
    "minecraft:zombie_villager",
    "minecraft:phantom",
    "minecraft:ravager",
    "minecraft:piglin",
];
const PROJECTILES: &[&str] = [
    "minecraft:arrow",
    "minecraft:dragon_fireball",
    "minecraft:firework_rocket",
    "minecraft:fireball",
    "minecraft:llama_spit",
    "minecraft:small_fireball",
    "minecraft:snowball",
    "minecraft:spectral_arrow",
    "minecraft:egg",
    "minecraft:ender_pearl",
    "minecraft:experience_bottle",
    "minecraft:potion",
    "minecraft:trident",
    "minecraft:wither_skull",
];

/// `EntityUUIDFix`.
fn entity_uuids(cx: &mut Cx) {
    cx.each(Ty::Entity, |e| {
        let Some(id) = as_str(e.get("id")).map(|i| namespaced(&i)) else {
            // Without an id no choice matches; the UUID is still fixed.
            replace_least_most(e, "UUID", "UUID");
            return;
        };
        let id = id.as_str();
        replace_least_most(e, "UUID", "UUID");
        if ABSTRACT_HORSES.contains(&id) || TAMEABLE_ANIMALS.contains(&id) {
            animal_uuids(e);
            replace_uuid_string(e, "OwnerUUID", "Owner");
        }
        if ANIMALS.contains(&id) {
            animal_uuids(e);
        }
        if MOBS.contains(&id) {
            mob_uuids(e);
        }
        if id == "minecraft:armor_stand" {
            living_entity_uuids(e);
        }
        if PROJECTILES.contains(&id)
            && let Some(owner) = remove(e, "OwnerUUID")
        {
            set(e, "Owner", owner);
        }
        match id {
            "minecraft:bee" | "minecraft:zombified_piglin" => {
                replace_uuid_string(e, "HurtBy", "HurtBy");
            }
            "minecraft:fox" => {
                if e.get("TrustedUUIDs").is_some() {
                    let trusted: Vec<Tag> = stream(e.get("TrustedUUIDs"))
                        .into_iter()
                        .map(|t| uuid_from_longs(&t, "M", "L").unwrap_or(t))
                        .collect();
                    remove(e, "TrustedUUIDs");
                    set(e, "Trusted", Tag::List(trusted));
                }
            }
            "minecraft:item" => {
                replace_ml(e, "Owner", "Owner");
                replace_ml(e, "Thrower", "Thrower");
            }
            "minecraft:shulker_bullet" => {
                replace_ml(e, "Owner", "Owner");
                replace_ml(e, "Target", "Target");
            }
            "minecraft:area_effect_cloud" | "minecraft:evoker_fangs" => {
                replace_least_most(e, "OwnerUUID", "Owner");
            }
            "minecraft:zombie_villager" => {
                replace_least_most(e, "ConversionPlayer", "ConversionPlayer")
            }
            "minecraft:piglin" => {
                if let Some(angry) = e
                    .get_mut("Brain")
                    .and_then(|b| b.get_mut("memories"))
                    .and_then(|m| m.get_mut("minecraft:angry_at"))
                {
                    replace_uuid_string(angry, "value", "value");
                }
            }
            _ => {}
        }
    });
}

/// `BlockEntityUUIDFix`.
fn block_entity_uuids(cx: &mut Cx) {
    cx.each(Ty::BlockEntity, |be| {
        if id_is(be, "minecraft:conduit") {
            replace_ml(be, "target_uuid", "Target");
        }
        if id_is(be, "minecraft:skull")
            && let Some(mut owner) = remove(be, "Owner")
        {
            replace_uuid_string(&mut owner, "Id", "Id");
            set(be, "SkullOwner", owner);
        }
    });
}

/// `PlayerUUIDFix`.
fn player_uuids(cx: &mut Cx) {
    cx.each(Ty::Player, |p| {
        if let Some(vehicle) = p.get_mut("RootVehicle") {
            replace_least_most(vehicle, "Attach", "Attach");
        }
        living_entity_uuids(p);
        replace_least_most(p, "UUID", "UUID");
    });
}

/// `LevelUUIDFix`.
fn level_uuids(cx: &mut Cx) {
    cx.each(Ty::Level, |level| {
        if let Some(Tag::Compound(dimensions)) = level.get_mut("DimensionData") {
            for dimension in dimensions.values_mut() {
                if let Some(fight) = dimension.get_mut("DragonFight") {
                    replace_least_most(fight, "DragonUUID", "Dragon");
                }
            }
        }
        replace_uuid_string(level, "WanderingTraderId", "WanderingTraderId");
        if let Some(Tag::Compound(events)) = level.get_mut("CustomBossEvents") {
            for event in events.values_mut() {
                map_elements(event, "Players", |player| {
                    uuid_from_longs(&player, "M", "L").unwrap_or(player)
                });
            }
        }
    });
}

/// `SavedDataUUIDFix` (raids).
fn raids_uuids(cx: &mut Cx) {
    cx.each(Ty::SavedRaids, |saved| {
        let Some(data) = saved.get_mut("data") else {
            return;
        };
        map_elements(data, "Raids", |mut raid| {
            map_elements(&mut raid, "HeroesOfTheVillage", |hero| {
                uuid_from_longs(&hero, "UUIDMost", "UUIDLeast").unwrap_or(hero)
            });
            raid
        });
    });
}

/// `ItemStackUUIDFix`.
fn item_stack_uuids(cx: &mut Cx) {
    cx.each(Ty::ItemStack, |stack| {
        let head =
            as_str(stack.get("id")).is_some_and(|id| namespaced(&id) == "minecraft:player_head");
        let Some(tag) = stack.get_mut("tag") else {
            return;
        };
        map_elements(tag, "AttributeModifiers", |mut modifier| {
            replace_least_most(&mut modifier, "UUID", "UUID");
            modifier
        });
        if head && let Some(owner) = tag.get_mut("SkullOwner") {
            replace_uuid_string(owner, "Id", "Id");
        }
    });
}

/// `GossipUUIDFix`.
fn gossip_uuids(cx: &mut Cx, entity: &str) {
    cx.entity(entity, |e| {
        if let Some(gossips) = e.get_mut("Gossips")
            && crate::datafix::dynamic::is_list(Some(gossips))
        {
            *gossips = Tag::List(
                stream(Some(gossips))
                    .into_iter()
                    .map(|mut g| {
                        replace_least_most(&mut g, "Target", "Target");
                        g
                    })
                    .collect(),
            );
        }
    });
}

// ------------------------------------------------------------------ 2518 to 2535

/// `JigsawPropertiesFix`.
fn jigsaw_properties(cx: &mut Cx) {
    cx.block_entity("minecraft:jigsaw", |be| {
        let name = super::dynamic::as_string(be.get("attachement_type"), "minecraft:empty");
        let pool = super::dynamic::as_string(be.get("target_pool"), "minecraft:empty");
        set(be, "name", string(&name));
        set(be, "target", string(&name));
        remove(be, "attachement_type");
        set(be, "pool", string(&pool));
        remove(be, "target_pool");
    });
}

/// `LegacyJigsawRotationFix`.
fn jigsaw_rotation(cx: &mut Cx) {
    cx.each(Ty::BlockState, |state| {
        if as_str(state.get("Name")).is_none_or(|n| namespaced(&n) != "minecraft:jigsaw") {
            return;
        }
        if let Some(props) = state.get_mut("Properties") {
            let facing = super::dynamic::as_string(props.get("facing"), "north");
            let orientation = match facing.as_str() {
                "down" => "down_south",
                "up" => "up_north",
                "north" => "north_up",
                "south" => "south_up",
                "west" => "west_up",
                "east" => "east_up",
                other => other,
            };
            remove(props, "facing");
            set(props, "orientation", string(orientation));
        }
    });
}

const ATTRIBUTES: &[(&str, &str)] = &[
    ("generic.maxHealth", "minecraft:generic.max_health"),
    ("Max Health", "minecraft:generic.max_health"),
    (
        "zombie.spawnReinforcements",
        "minecraft:zombie.spawn_reinforcements",
    ),
    (
        "Spawn Reinforcements Chance",
        "minecraft:zombie.spawn_reinforcements",
    ),
    ("horse.jumpStrength", "minecraft:horse.jump_strength"),
    ("Jump Strength", "minecraft:horse.jump_strength"),
    ("generic.followRange", "minecraft:generic.follow_range"),
    ("Follow Range", "minecraft:generic.follow_range"),
    (
        "generic.knockbackResistance",
        "minecraft:generic.knockback_resistance",
    ),
    (
        "Knockback Resistance",
        "minecraft:generic.knockback_resistance",
    ),
    ("generic.movementSpeed", "minecraft:generic.movement_speed"),
    ("Movement Speed", "minecraft:generic.movement_speed"),
    ("generic.flyingSpeed", "minecraft:generic.flying_speed"),
    ("Flying Speed", "minecraft:generic.flying_speed"),
    ("generic.attackDamage", "minecraft:generic.attack_damage"),
    (
        "generic.attackKnockback",
        "minecraft:generic.attack_knockback",
    ),
    ("generic.attackSpeed", "minecraft:generic.attack_speed"),
    (
        "generic.armorToughness",
        "minecraft:generic.armor_toughness",
    ),
];

/// `BitStorageAlignFix.addPadding`: values stored without crossing longs.
pub(super) fn add_padding(size: usize, bits: usize, data: &[i64]) -> Vec<i64> {
    let length = data.len();
    if length == 0 {
        return Vec::new();
    }
    let mask = (1i64 << bits) - 1;
    let per_long = 64 / bits;
    let required = size.div_ceil(per_long);
    let mut result = vec![0i64; required];
    let mut out_index = 0;
    let mut out_start = 0;
    let mut out_data = 0i64;
    let mut current_index = 0;
    let mut current = data[0];
    let mut next = if length > 1 { data[1] } else { 0 };
    for index in 0..size {
        let position = index * bits;
        let start_data = position >> 6;
        let end_data = ((index + 1) * bits - 1) >> 6;
        let start_bit = position ^ (start_data << 6);
        if start_data != current_index {
            current = next;
            next = if start_data + 1 < length {
                data[start_data + 1]
            } else {
                0
            };
            current_index = start_data;
        }
        let value = if start_data == end_data {
            ((current as u64) >> start_bit) as i64 & mask
        } else {
            let shift = 64 - start_bit;
            ((((current as u64) >> start_bit) as i64) | next.wrapping_shl(shift as u32)) & mask
        };
        let out_end = out_start + bits;
        if out_end >= 64 {
            result[out_index] = out_data;
            out_index += 1;
            out_data = value;
            out_start = bits;
        } else {
            out_data |= value << out_start;
            out_start = out_end;
        }
    }
    if out_data != 0 {
        result[out_index] = out_data;
    }
    result
}

fn align_storage(tag: &mut Tag, field: &str, size: usize, bits: usize) {
    if let Some(v) = tag.get_mut(field) {
        let input = as_long_stream(Some(v)).unwrap_or_default();
        *v = Tag::LongArray(add_padding(size, bits, &input));
    }
}

/// `BitStorageAlignFix`: block states and heightmaps no longer cross longs.
fn bit_storage_align(cx: &mut Cx) {
    cx.each(Ty::Chunk, |c| {
        let Some(level) = c.get_mut("Level") else {
            return;
        };
        if let Some(Tag::List(sections)) = level.get_mut("Sections") {
            for section in sections {
                let Some(Tag::List(palette)) = section.get("Palette") else {
                    continue;
                };
                let bits = 4.max(ceillog2(palette.len())) as usize;
                if bits & (bits - 1) != 0 {
                    align_storage(section, "BlockStates", 4096, bits);
                }
            }
        }
        if let Some(Tag::Compound(heightmaps)) = level.get_mut("Heightmaps") {
            for heightmap in heightmaps.values_mut() {
                let input = as_long_stream(Some(heightmap)).unwrap_or_default();
                *heightmap = Tag::LongArray(add_padding(256, 9, &input));
            }
        }
    });
}

/// `StriderGravityFix`.
fn strider_gravity(cx: &mut Cx) {
    cx.entity("minecraft:strider", |e| {
        if as_bool(e.get("NoGravity"), false) {
            set(e, "NoGravity", Tag::Byte(0));
        }
    });
}

/// `RedstoneWireConnectionsFix`.
fn redstone_wire(cx: &mut Cx) {
    cx.each(Ty::BlockState, |state| {
        if as_str(state.get("Name")).as_deref() != Some("minecraft:redstone_wire") {
            return;
        }
        let Some(props) = state.get_mut("Properties") else {
            return;
        };
        let side = |props: &Tag, name: &str| super::dynamic::as_string(props.get(name), "none");
        let (east, west, north, south) = (
            side(props, "east"),
            side(props, "west"),
            side(props, "north"),
            side(props, "south"),
        );
        let connected = |s: &str| s != "none";
        let east_west = connected(&east) || connected(&west);
        let north_south = connected(&north) || connected(&south);
        let fixed = [
            (
                "east",
                if !connected(&east) && !north_south {
                    "side"
                } else {
                    &east
                },
            ),
            (
                "west",
                if !connected(&west) && !north_south {
                    "side"
                } else {
                    &west
                },
            ),
            (
                "north",
                if !connected(&north) && !east_west {
                    "side"
                } else {
                    &north
                },
            ),
            (
                "south",
                if !connected(&south) && !east_west {
                    "side"
                } else {
                    &south
                },
            ),
        ];
        for (name, value) in fixed {
            if props.get(name).is_some() {
                set(props, name, string(value));
            }
        }
    });
}

/// `VillagerFollowRangeFix`.
fn villager_follow_range(cx: &mut Cx) {
    cx.entity("minecraft:villager", |e| {
        map_elements(e, "Attributes", |mut attribute| {
            if super::dynamic::as_string(attribute.get("Name"), "") == "generic.follow_range"
                && as_double(attribute.get("Base"), 0.0) == 16.0
            {
                set(&mut attribute, "Base", Tag::Double(48.0f64.to_bits()));
            }
            attribute
        });
    });
}

/// `EntityShulkerRotationFix`.
fn shulker_rotation(cx: &mut Cx) {
    cx.entity("minecraft:shulker", |e| {
        let mut rotation: Vec<f64> = stream(e.get("Rotation"))
            .iter()
            .map(|d| as_double(Some(d), 180.0))
            .collect();
        if !rotation.is_empty() {
            rotation[0] -= 180.0;
            set(
                e,
                "Rotation",
                Tag::List(
                    rotation
                        .into_iter()
                        .map(|d| Tag::Double(d.to_bits()))
                        .collect(),
                ),
            );
        }
    });
}
