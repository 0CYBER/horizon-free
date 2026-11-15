//! An older client loads the synchronised registries from its own built-in data (#15138): the server's
//! `minecraft:core` pack is offered in the client's release ([`core_pack_down`] / [`core_pack_up`]), so
//! registry entries go without data. A registry the client's release doesn't have is not sent, and the
//! entries its release lacks leave their registry. A synchronised registry is numbered by the order of the
//! entries sent, so every id sent afterwards into a registry that lost entries (chunk biomes, tags, damage
//! events, item enchantments / trims / banner layers / painting variants, painting entity data) is
//! renumbered here; a removed entry becomes a stand-in or, where a list can do without it, is left out.
//! The tables come from the vanilla jars (`tools/generate_sync_registry_adapter.py`, `sync_registries.rs`).
use std::collections::HashMap;

use super::sync_registries as tables;
use crate::{
    Result,
    adapter::ServerVersion,
    buf::{Codec, Reader, Writer},
    chunk::{self, Paletted, Section, Strategy},
    packets::{configuration, play},
};

pub(super) const BIOMES: &str = "minecraft:worldgen/biome";
pub(super) const DAMAGE_TYPE: &str = "minecraft:damage_type";
pub(super) const ENCHANTMENT: &str = "minecraft:enchantment";
pub(super) const PAINTING_VARIANT: &str = "minecraft:painting_variant";

/// The core pack names a client on `version`'s protocol may have (one protocol can be several releases;
/// the client answers with the one it has). Empty: no own core pack (the pinned version, and releases
/// before 1.20.5, which have no known packs).
fn releases(version: ServerVersion) -> &'static [&'static str] {
    match version {
        ServerVersion::V26_2 => &["26.2"],
        ServerVersion::V26_1 => &["26.1", "26.1.1", "26.1.2"],
        ServerVersion::V1_21_11 => &["1.21.11"],
        ServerVersion::V1_21_10 => &["1.21.9", "1.21.10"],
        ServerVersion::V1_21_8 => &["1.21.7", "1.21.8"],
        ServerVersion::V1_21_6 => &["1.21.6"],
        ServerVersion::V1_21_5 => &["1.21.5"],
        ServerVersion::V1_21_4 => &["1.21.4"],
        ServerVersion::V1_21_3 => &["1.21.2", "1.21.3"],
        ServerVersion::V1_21_1 => &["1.21", "1.21.1"],
        ServerVersion::V1_20_6 => &["1.20.5", "1.20.6"],
        ServerVersion::Current | ServerVersion::V1_20_3 | ServerVersion::V1_20_2 => &[],
    }
}

/// The tables of one release on `version`'s protocol. The releases sharing a protocol have the same
/// synchronised registries with the same entries (every `data/minecraft/<registry>/` of their server jars
/// compared: 1.20.5/1.20.6, 1.21/1.21.1, 1.21.2/1.21.3, 1.21.7/1.21.8, 1.21.9/1.21.10, 26.1/26.1.1/26.1.2),
/// so any one of them stands for all.
fn table(
    version: ServerVersion,
) -> Option<(
    &'static [&'static str],
    &'static [(&'static str, &'static [&'static str])],
)> {
    Some(match version {
        ServerVersion::V26_2 => (tables::v26_2::ABSENT, tables::v26_2::LACKING),
        ServerVersion::V26_1 => (tables::v26_1_2::ABSENT, tables::v26_1_2::LACKING),
        ServerVersion::V1_21_11 => (tables::v1_21_11::ABSENT, tables::v1_21_11::LACKING),
        ServerVersion::V1_21_10 => (tables::v1_21_9::ABSENT, tables::v1_21_9::LACKING),
        ServerVersion::V1_21_8 => (tables::v1_21_8::ABSENT, tables::v1_21_8::LACKING),
        ServerVersion::V1_21_6 => (tables::v1_21_6::ABSENT, tables::v1_21_6::LACKING),
        ServerVersion::V1_21_5 => (tables::v1_21_5::ABSENT, tables::v1_21_5::LACKING),
        ServerVersion::V1_21_4 => (tables::v1_21_4::ABSENT, tables::v1_21_4::LACKING),
        ServerVersion::V1_21_3 => (tables::v1_21_3::ABSENT, tables::v1_21_3::LACKING),
        ServerVersion::V1_21_1 => (tables::v1_21_1::ABSENT, tables::v1_21_1::LACKING),
        ServerVersion::V1_20_6 => (tables::v1_20_6::ABSENT, tables::v1_20_6::LACKING),
        _ => return None,
    })
}

/// What a client is shown in place of a removed entry; `None`: the entry is left out where it is listed
/// (an enchantment, a banner layer, a tag member), or the first entry where one id is needed.
fn stand_in(registry: &str, entry: &str) -> Option<&'static str> {
    Some(match (registry, entry) {
        (BIOMES, "minecraft:dappled_forest") => "minecraft:forest",
        (BIOMES, "minecraft:pale_garden") => "minecraft:dark_forest",
        (BIOMES, "minecraft:sulfur_caves") => "minecraft:dripstone_caves",
        (BIOMES, _) => "minecraft:plains",
        (DAMAGE_TYPE, _) => "minecraft:generic",
        (PAINTING_VARIANT, _) => "minecraft:kebab",
        ("minecraft:trim_pattern", _) => "minecraft:sentry",
        ("minecraft:trim_material", _) => "minecraft:iron",
        _ => return None,
    })
}

/// Whether the client is offered `minecraft:core` in its own release.
pub(super) fn own_core_pack(version: ServerVersion) -> bool {
    !releases(version).is_empty() && table(version).is_some()
}

/// `select_known_packs` to the client: the pinned core pack becomes the client's releases.
pub(super) fn core_pack_down(version: ServerVersion, payload: &[u8]) -> Result<Vec<u8>> {
    let mut p = configuration::SelectKnownPacks::read(&mut Reader::new(payload))?;
    let mut packs = Vec::with_capacity(p.packs.len() + 2);
    for pack in p.packs {
        if pack.namespace == "minecraft" && pack.id == "core" && pack.version == crate::GAME_VERSION
        {
            for name in releases(version) {
                let mut own = pack.clone();
                own.version = (*name).into();
                packs.push(own);
            }
        } else {
            packs.push(pack);
        }
    }
    p.packs = packs;
    let mut w = Writer::new();
    configuration::SelectKnownPacks::write(&p, &mut w);
    Ok(w.data)
}

/// The client's `select_known_packs` reply: its release's core pack becomes the pinned one again.
pub(super) fn core_pack_up(version: ServerVersion, payload: &[u8]) -> Result<Vec<u8>> {
    let mut p = configuration::SelectKnownPacks::read(&mut Reader::new(payload))?;
    for pack in &mut p.packs {
        if pack.namespace == "minecraft"
            && pack.id == "core"
            && releases(version).contains(&pack.version.as_str())
        {
            pack.version = crate::GAME_VERSION.into();
        }
    }
    let mut w = Writer::new();
    configuration::SelectKnownPacks::write(&p, &mut w);
    Ok(w.data)
}

/// Whether `registry` is a synchronised registry `version` doesn't have at all.
pub(super) fn absent(version: ServerVersion, registry: &str) -> bool {
    table(version).is_some_and(|(absent, _)| absent.contains(&registry))
}

/// Whether `payload` (a `registry_data`) is such a registry: it is not sent.
pub(super) fn absent_registry(version: ServerVersion, payload: &[u8]) -> Result<bool> {
    if table(version).is_none() {
        return Ok(false);
    }
    let p = configuration::RegistryData::read(&mut Reader::new(payload))?;
    Ok(absent(version, &p.registry))
}

/// 1.20.5 and 1.20.6 have enchantments and painting variants built in, not synchronised: no registry data is
/// sent for them (they are in the release's `ABSENT`), and the client numbers them in its own fixed order
/// (vanilla 1.20.5/1.20.6 `registries.json` report, `protocol_id` order; both releases agree).
const ENCHANTMENTS_1_20_5: &[&str] = &[
    "minecraft:protection",
    "minecraft:fire_protection",
    "minecraft:feather_falling",
    "minecraft:blast_protection",
    "minecraft:projectile_protection",
    "minecraft:respiration",
    "minecraft:aqua_affinity",
    "minecraft:thorns",
    "minecraft:depth_strider",
    "minecraft:frost_walker",
    "minecraft:binding_curse",
    "minecraft:soul_speed",
    "minecraft:swift_sneak",
    "minecraft:sharpness",
    "minecraft:smite",
    "minecraft:bane_of_arthropods",
    "minecraft:knockback",
    "minecraft:fire_aspect",
    "minecraft:looting",
    "minecraft:sweeping_edge",
    "minecraft:efficiency",
    "minecraft:silk_touch",
    "minecraft:unbreaking",
    "minecraft:fortune",
    "minecraft:power",
    "minecraft:punch",
    "minecraft:flame",
    "minecraft:infinity",
    "minecraft:luck_of_the_sea",
    "minecraft:lure",
    "minecraft:loyalty",
    "minecraft:impaling",
    "minecraft:riptide",
    "minecraft:channeling",
    "minecraft:multishot",
    "minecraft:quick_charge",
    "minecraft:piercing",
    "minecraft:density",
    "minecraft:breach",
    "minecraft:wind_burst",
    "minecraft:mending",
    "minecraft:vanishing_curse",
];
const PAINTING_VARIANTS_1_20_5: &[&str] = &[
    "minecraft:kebab",
    "minecraft:aztec",
    "minecraft:alban",
    "minecraft:aztec2",
    "minecraft:bomb",
    "minecraft:plant",
    "minecraft:wasteland",
    "minecraft:pool",
    "minecraft:courbet",
    "minecraft:sea",
    "minecraft:sunset",
    "minecraft:creebet",
    "minecraft:wanderer",
    "minecraft:graham",
    "minecraft:match",
    "minecraft:bust",
    "minecraft:stage",
    "minecraft:void",
    "minecraft:skull_and_roses",
    "minecraft:wither",
    "minecraft:fighters",
    "minecraft:pointer",
    "minecraft:pigscene",
    "minecraft:burning_skull",
    "minecraft:skeleton",
    "minecraft:earth",
    "minecraft:wind",
    "minecraft:water",
    "minecraft:fire",
    "minecraft:donkey_kong",
];

/// For a registry `version` has built in: the pinned ids (the order of the entries the server names in
/// `payload`, a `registry_data`) as the client numbers them, -1 for an entry it lacks. A 1.20.6 client
/// left with "No value with id 42" over the first item that named an enchantment in 26.3's order.
pub(super) fn built_in(version: ServerVersion, payload: &[u8]) -> Result<Option<(String, IdMap)>> {
    if version != ServerVersion::V1_20_6 {
        return Ok(None);
    }
    let p = configuration::RegistryData::read(&mut Reader::new(payload))?;
    let own = match p.registry.as_str() {
        ENCHANTMENT => ENCHANTMENTS_1_20_5,
        PAINTING_VARIANT => PAINTING_VARIANTS_1_20_5,
        _ => return Ok(None),
    };
    let down = p
        .entries
        .iter()
        .map(|e| {
            own.iter()
                .position(|o| *o == e.id)
                .or_else(|| {
                    stand_in(&p.registry, &e.id).and_then(|s| own.iter().position(|o| *o == s))
                })
                .map_or(-1, |i| i as i32)
        })
        .collect();
    // The client's index -> the pinned one: where the server names the same entry (the first if it does not).
    let up = own
        .iter()
        .map(|o| {
            p.entries
                .iter()
                .position(|e| e.id == *o)
                .map_or(0, |i| i as i32)
        })
        .collect();
    Ok(Some((p.registry, IdMap { down, up })))
}

/// One registry's renumbering.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct IdMap {
    /// Pinned index -> the client's, -1 for a removed entry without a stand-in.
    pub(super) down: Vec<i32>,
    /// The client's index -> the pinned one (the entries kept, in order).
    pub(super) up: Vec<i32>,
}

impl IdMap {
    /// The client's id; `None`: removed without a stand-in. Ids past the table pass unchanged.
    pub(super) fn down(&self, id: i32) -> Option<i32> {
        match usize::try_from(id).ok().and_then(|i| self.down.get(i)) {
            Some(&-1) => None,
            Some(&to) => Some(to),
            None => Some(id),
        }
    }
    /// The pinned id of the client's `id`. Ids past the table pass unchanged.
    pub(super) fn up(&self, id: i32) -> i32 {
        usize::try_from(id)
            .ok()
            .and_then(|i| self.up.get(i))
            .copied()
            .unwrap_or(id)
    }
    /// The client's id, the first entry for a removed one without a stand-in.
    pub(super) fn down_or_first(&self, id: i32) -> i32 {
        self.down(id).unwrap_or(0)
    }
    /// Bits of the client's global biome palette (`Mth.ceillog2` of its biome count).
    fn global_bits(&self) -> u8 {
        let n = self.down.iter().copied().max().map_or(1, |m| m + 1).max(1) as u32;
        (32 - (n - 1).leading_zeros()) as u8
    }
    fn container(&self, p: &Paletted) -> Paletted {
        if !p.palette.is_empty() {
            let mut out = p.clone();
            out.palette
                .iter_mut()
                .for_each(|b| *b = self.down_or_first(*b));
            return out;
        }
        let values: Vec<i32> = (0..chunk::BIOMES)
            .map(|i| self.down_or_first(p.get(i)))
            .collect();
        Paletted::from_values(&values, Strategy::Biomes, self.global_bits())
    }
}

/// Every renumbered registry of one connection.
pub(super) type IdMaps = HashMap<String, IdMap>;

impl crate::adapter::ids::SyncedIds for IdMaps {
    fn down(&self, registry: &str, id: i32) -> Option<i32> {
        match self.get(registry) {
            Some(map) => map.down(id),
            None => Some(id),
        }
    }
    fn up(&self, registry: &str, id: i32) -> i32 {
        match self.get(registry) {
            Some(map) => map.up(id),
            None => id,
        }
    }
}

/// The registry without the entries `version` lacks, and the renumbering that follows from it.
/// `None`: nothing removed.
pub(super) fn registry_data(
    version: ServerVersion,
    payload: &[u8],
) -> Result<Option<(Vec<u8>, String, IdMap)>> {
    let Some((_, lacking)) = table(version) else {
        return Ok(None);
    };
    let mut p = configuration::RegistryData::read(&mut Reader::new(payload))?;
    let Some((_, removed)) = lacking.iter().find(|(r, _)| *r == p.registry) else {
        return Ok(None);
    };
    if !p.entries.iter().any(|e| removed.contains(&e.id.as_str())) {
        return Ok(None);
    }
    let kept: Vec<&str> = p
        .entries
        .iter()
        .map(|e| e.id.as_str())
        .filter(|id| !removed.contains(id))
        .collect();
    let index = |name: &str| kept.iter().position(|k| *k == name).map(|i| i as i32);
    let map = p
        .entries
        .iter()
        .map(|e| {
            if removed.contains(&e.id.as_str()) {
                // A stand-in a data pack took away is the first entry instead.
                stand_in(&p.registry, &e.id).map_or(-1, |s| index(s).unwrap_or(0))
            } else {
                index(&e.id).unwrap_or(0)
            }
        })
        .collect();
    let up = (0..p.entries.len() as i32)
        .filter(|&i| !removed.contains(&p.entries[i as usize].id.as_str()))
        .collect();
    let registry = p.registry.clone();
    p.entries.retain(|e| !removed.contains(&e.id.as_str()));
    let mut w = Writer::new();
    configuration::RegistryData::write(&p, &mut w);
    Ok(Some((w.data, registry, IdMap { down: map, up })))
}

/// Tags a release's own registry data names that 26.3 has under another name: (registry, the release's
/// tag, 26.3's tag with the same members). A client from 1.21.2 on reads its built-in enchantments against
/// the tags sent in configuration and leaves with "Failed to load registries" over a tag that is missing
/// ("Missing tag: 'minecraft:enchantable/sword' in 'minecraft:item'"). Through 1.21.10 the swords' tag was
/// `enchantable/sword` (`#minecraft:swords`); 1.21.11 named it `enchantable/sweeping`.
fn renamed_tags(version: ServerVersion) -> &'static [(&'static str, &'static str, &'static str)] {
    if version != ServerVersion::Current && version.at_most(ServerVersion::V1_21_10) {
        &[(
            "minecraft:item",
            "minecraft:enchantable/sword",
            "minecraft:enchantable/sweeping",
        )]
    } else {
        &[]
    }
}

/// Tags of registries the client doesn't have are left out; the members of renumbered registries take the
/// client's ids (a removed member is left out, duplicates a stand-in makes are kept once). A tag the
/// client's release knows by an older name is sent under that name too ([`renamed_tags`]).
pub(super) fn tags(
    version: ServerVersion,
    maps: &IdMaps,
    registries: &mut Vec<configuration::RegistryTags>,
) {
    registries.retain(|r| !absent(version, &r.registry));
    for (registry, old, new) in renamed_tags(version) {
        let Some(r) = registries.iter_mut().find(|r| r.registry == *registry) else {
            continue;
        };
        if r.tags.iter().any(|t| t.name == *old) {
            continue;
        }
        if let Some(ids) = r
            .tags
            .iter()
            .find(|t| t.name == *new)
            .map(|t| t.ids.clone())
        {
            r.tags.push(configuration::TagEntry {
                name: (*old).into(),
                ids,
            });
        }
    }
    for registry in registries.iter_mut() {
        let Some(map) = maps.get(&registry.registry) else {
            continue;
        };
        for tag in &mut registry.tags {
            let mut ids: Vec<i32> = Vec::with_capacity(tag.ids.len());
            for id in tag.ids.iter().filter_map(|&id| map.down(id)) {
                if !ids.contains(&id) {
                    ids.push(id);
                }
            }
            tag.ids = ids;
        }
    }
}

/// A column's sections with their biomes renumbered (pinned layout otherwise; `block_bits` is the pinned
/// global block state width).
pub(super) fn level_chunk(map: &IdMap, block_bits: u8, payload: &[u8]) -> Result<Vec<u8>> {
    let mut p = play::LevelChunkWithLight::read(&mut Reader::new(payload))?;
    let mut r = Reader::new(&p.chunk.buffer);
    let mut sections = Vec::new();
    while r.remaining() > 0 {
        let mut s = Section::read(&mut r, block_bits, 0)?;
        s.biomes = map.container(&s.biomes);
        sections.push(s);
    }
    p.chunk.buffer = chunk::ChunkData::from_sections(&sections);
    let mut w = Writer::new();
    play::LevelChunkWithLight::write(&p, &mut w);
    Ok(w.data)
}

/// `chunks_biomes` with every column's biome containers renumbered.
pub(super) fn chunks_biomes(map: &IdMap, payload: &[u8]) -> Result<Vec<u8>> {
    let mut p = play::ChunksBiomes::read(&mut Reader::new(payload))?;
    for c in &mut p.chunks {
        let mut r = Reader::new(&c.buffer);
        let mut biomes = Vec::new();
        while r.remaining() > 0 {
            biomes.push(map.container(&Paletted::read(&mut r, Strategy::Biomes, 0)?));
        }
        *c = play::ChunkBiomeData::from_biomes(c.pos, &biomes);
    }
    let mut w = Writer::new();
    play::ChunksBiomes::write(&p, &mut w);
    Ok(w.data)
}

/// `damage_event`: the source's damage type renumbered (VarInt entity id, VarInt type, then as it is).
pub(super) fn damage_event(map: &IdMap, payload: &[u8]) -> Result<Vec<u8>> {
    let mut r = Reader::new(payload);
    let entity = r.varint()?;
    let kind = r.varint()?;
    let mut w = Writer::new();
    w.varint(entity);
    w.varint(map.down_or_first(kind));
    w.bytes(r.rest());
    Ok(w.data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packets::configuration::{KnownPack, RegistryEntry, SelectKnownPacks};

    fn encode<T: Codec<T>>(v: &T) -> Vec<u8> {
        let mut w = Writer::new();
        T::write(v, &mut w);
        w.data
    }

    fn core(version: &str) -> KnownPack {
        KnownPack {
            namespace: "minecraft".into(),
            id: "core".into(),
            version: version.into(),
        }
    }

    #[test]
    fn a_client_is_offered_its_own_releases_core_packs_and_its_answer_goes_back_up() {
        let offer = SelectKnownPacks {
            packs: vec![core(crate::GAME_VERSION)],
        };
        let down = core_pack_down(ServerVersion::V1_21_1, &encode(&offer)).unwrap();
        let seen = SelectKnownPacks::read(&mut Reader::new(&down)).unwrap();
        assert_eq!(seen.packs, vec![core("1.21"), core("1.21.1")]);
        // a 1.21 client answers with its own
        let answer = encode(&SelectKnownPacks {
            packs: vec![core("1.21")],
        });
        let up = core_pack_up(ServerVersion::V1_21_1, &answer).unwrap();
        assert_eq!(
            SelectKnownPacks::read(&mut Reader::new(&up)).unwrap(),
            offer
        );
        for v in ServerVersion::ALL {
            let expected = !matches!(
                v,
                ServerVersion::Current | ServerVersion::V1_20_3 | ServerVersion::V1_20_2
            );
            assert_eq!(own_core_pack(v), expected, "{v:?}");
        }
    }

    fn registry(name: &str, entries: &[&str]) -> Vec<u8> {
        encode(&configuration::RegistryData {
            registry: name.into(),
            entries: entries
                .iter()
                .map(|n| RegistryEntry {
                    id: (*n).into(),
                    data: None,
                })
                .collect(),
        })
    }

    #[test]
    fn dappled_forest_leaves_a_26_2_clients_biomes_and_the_ids_after_it_move_down() {
        let biomes = [
            "minecraft:badlands",
            "minecraft:dappled_forest",
            "minecraft:forest",
            "minecraft:the_void",
        ];
        let (out, name, map) = registry_data(ServerVersion::V26_2, &registry(BIOMES, &biomes))
            .unwrap()
            .unwrap();
        assert_eq!(name, BIOMES);
        let sent = configuration::RegistryData::read(&mut Reader::new(&out)).unwrap();
        let names: Vec<&str> = sent.entries.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(
            names,
            [
                "minecraft:badlands",
                "minecraft:forest",
                "minecraft:the_void"
            ]
        );
        assert_eq!(map.down, [0, 1, 1, 2]);
        assert_eq!(map.up, [0, 2, 3]);
        assert!(
            registry_data(ServerVersion::Current, &registry(BIOMES, &biomes))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn an_enchantment_a_release_lacks_is_left_out_and_damage_types_get_generic() {
        // 1.21.1 lacks one enchantment (the generated table) and some damage types
        let lacking = |reg: &str| {
            table(ServerVersion::V1_21_1)
                .unwrap()
                .1
                .iter()
                .find(|(r, _)| *r == reg)
                .unwrap()
                .1
        };
        let gone = lacking(ENCHANTMENT)[0];
        let (_, _, map) = registry_data(
            ServerVersion::V1_21_1,
            &registry(
                ENCHANTMENT,
                &["minecraft:aqua_affinity", gone, "minecraft:zz_last"],
            ),
        )
        .unwrap()
        .unwrap();
        assert_eq!(map.down, [0, -1, 1]);
        assert_eq!(map.down(1), None);
        assert_eq!(map.up, [0, 2]);
        assert_eq!(map.up(1), 2);
        let gone = lacking(DAMAGE_TYPE)[0];
        let (_, _, map) = registry_data(
            ServerVersion::V1_21_1,
            &registry(DAMAGE_TYPE, &["minecraft:arrow", gone, "minecraft:generic"]),
        )
        .unwrap()
        .unwrap();
        assert_eq!(map.down, [0, 1, 1]);
        assert_eq!(map.up, [0, 2]);
        let mut w = Writer::new();
        w.varint(7);
        w.varint(1);
        w.varint(0);
        let out = damage_event(&map, &w.data).unwrap();
        assert_eq!(out, vec![7, 1, 0]);
    }

    #[test]
    fn registries_a_release_does_not_synchronise_are_not_sent_and_their_tags_go_too() {
        assert!(
            absent_registry(
                ServerVersion::V26_2,
                &registry("minecraft:decorated_pot_pattern", &[])
            )
            .unwrap()
        );
        assert!(!absent_registry(ServerVersion::V26_2, &registry(BIOMES, &[])).unwrap());
        assert!(
            absent_registry(
                ServerVersion::V1_21_4,
                &registry("minecraft:pig_variant", &[])
            )
            .unwrap()
        );
        assert!(
            !absent_registry(
                ServerVersion::Current,
                &registry("minecraft:decorated_pot_pattern", &[])
            )
            .unwrap()
        );
        let mut maps = IdMaps::new();
        maps.insert(
            BIOMES.into(),
            IdMap {
                down: vec![0, 1, 1, 2],
                up: vec![0, 2, 3],
            },
        );
        let mut regs = vec![
            configuration::RegistryTags {
                registry: "minecraft:pig_variant".into(),
                tags: vec![],
            },
            configuration::RegistryTags {
                registry: BIOMES.into(),
                tags: vec![configuration::TagEntry {
                    name: "minecraft:is_forest".into(),
                    ids: vec![1, 2, 3],
                }],
            },
        ];
        tags(ServerVersion::V1_21_4, &maps, &mut regs);
        assert_eq!(regs.len(), 1);
        assert_eq!(regs[0].tags[0].ids, [1, 2]);
    }

    #[test]
    fn biome_containers_take_the_clients_numbering() {
        let map = IdMap {
            down: vec![0, 1, 1, 2],
            up: vec![0, 2, 3],
        };
        let single = Paletted::from_values(&[3; chunk::BIOMES], Strategy::Biomes, 2);
        assert_eq!(map.container(&single).get(0), 2);
        let mixed: Vec<i32> = (0..chunk::BIOMES as i32).map(|i| i % 4).collect();
        let down = map.container(&Paletted::from_values(&mixed, Strategy::Biomes, 2));
        assert_eq!(
            (0..4).map(|i| down.get(i)).collect::<Vec<_>>(),
            [0, 1, 1, 2]
        );
    }

    #[test]
    fn a_tag_26_3_renamed_is_sent_under_the_name_the_clients_release_has() {
        let item_tags = || {
            vec![configuration::RegistryTags {
                registry: "minecraft:item".into(),
                tags: vec![
                    configuration::TagEntry {
                        name: "minecraft:enchantable/sweeping".into(),
                        ids: vec![901, 906, 911],
                    },
                    configuration::TagEntry {
                        name: "minecraft:swords".into(),
                        ids: vec![901, 906, 911],
                    },
                ],
            }]
        };
        // 1.21.2 to 1.21.10 read their own enchantments against `enchantable/sword` and leave without it.
        for version in [
            ServerVersion::V1_21_10,
            ServerVersion::V1_21_4,
            ServerVersion::V1_21_3,
            ServerVersion::V1_21_1,
            ServerVersion::V1_20_6,
        ] {
            let mut regs = item_tags();
            tags(version, &IdMaps::new(), &mut regs);
            let names: Vec<&str> = regs[0].tags.iter().map(|t| t.name.as_str()).collect();
            assert_eq!(
                names,
                [
                    "minecraft:enchantable/sweeping",
                    "minecraft:swords",
                    "minecraft:enchantable/sword"
                ]
            );
            assert_eq!(regs[0].tags[2].ids, vec![901, 906, 911]);
        }
        // 1.21.11 and later have the tag under 26.3's name: nothing is added.
        for version in [
            ServerVersion::V1_21_11,
            ServerVersion::V26_1,
            ServerVersion::V26_2,
            ServerVersion::Current,
        ] {
            let mut regs = item_tags();
            tags(version, &IdMaps::new(), &mut regs);
            assert_eq!(regs[0].tags.len(), 2);
        }
        // A data pack that already defines the old name keeps its own definition.
        let mut regs = item_tags();
        regs[0].tags.push(configuration::TagEntry {
            name: "minecraft:enchantable/sword".into(),
            ids: vec![5],
        });
        tags(ServerVersion::V1_21_4, &IdMaps::new(), &mut regs);
        assert_eq!(regs[0].tags.len(), 3);
        assert_eq!(regs[0].tags[2].ids, vec![5]);
    }

    #[test]
    fn a_1_20_6_client_gets_enchantments_and_paintings_in_its_built_in_numbering() {
        // 26.3 sends enchantments in its registry's order; 1.20.6 numbers its built-in ones differently and
        // has neither lunge nor an entry past 41.
        let sent = registry(
            ENCHANTMENT,
            &[
                "minecraft:aqua_affinity",
                "minecraft:lunge",
                "minecraft:mending",
                "minecraft:wind_burst",
            ],
        );
        let (name, map) = built_in(ServerVersion::V1_20_6, &sent).unwrap().unwrap();
        assert_eq!(name, "minecraft:enchantment");
        assert_eq!(map.down, vec![6, -1, 40, 39]);
        // What the client sends comes back: its mending (40) is the server's third entry.
        assert_eq!(map.up(40), 2);
        assert_eq!(map.up(6), 0);
        assert_eq!(map.up(39), 3);
        assert_eq!(map.down(1), None);
        let sent = registry(
            PAINTING_VARIANT,
            &[
                "minecraft:alban",
                "minecraft:kebab",
                "minecraft:donkey_kong",
            ],
        );
        let (_, map) = built_in(ServerVersion::V1_20_6, &sent).unwrap().unwrap();
        assert_eq!(map.down, vec![2, 0, 29]);
        // Later releases synchronise both registries: no built-in table.
        assert_eq!(built_in(ServerVersion::V1_21_1, &sent).unwrap(), None);
        assert_eq!(
            built_in(
                ServerVersion::V1_20_6,
                &registry(BIOMES, &["minecraft:plains"])
            )
            .unwrap(),
            None
        );
    }
}
