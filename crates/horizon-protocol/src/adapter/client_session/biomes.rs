//! An older client loads the synchronised registries from its own built-in data (#15138): the server's
//! `minecraft:core` pack is offered in the client's release ([`core_pack_down`] / [`core_pack_up`]), so
//! registry entries go without data, and the entries the client's release lacks leave the biome registry.
//! Biomes are numbered by the order of that registry's entries, so every biome id sent afterwards (chunk
//! sections, `chunks_biomes`, biome tags) is renumbered here, a removed biome becoming a stand-in.
use crate::{
    Result,
    adapter::ServerVersion,
    buf::{Codec, Reader, Writer},
    chunk::{self, Paletted, Section, Strategy},
    packets::{configuration, play},
};

const BIOMES: &str = "minecraft:worldgen/biome";

/// Biomes the pinned release has and `version` lacks, with the biome the client is shown instead.
/// 26.2: the synchronised registries of both vanilla releases compared, `dappled_forest` is the only
/// new entry outside the three registries 26.2 doesn't synchronise (`layout_26_2::NEW_REGISTRIES`).
fn removed(version: ServerVersion) -> &'static [(&'static str, &'static str)] {
    match version {
        ServerVersion::V26_2 => &[("minecraft:dappled_forest", "minecraft:forest")],
        _ => &[],
    }
}

/// Whether the client is offered `minecraft:core` in its own release (verified releases only: an older
/// one whose registries differ in more than [`removed`] knows would still miss entries).
pub(super) fn own_core_pack(version: ServerVersion) -> bool {
    version == ServerVersion::V26_2
}

/// Pinned biome index -> the client's.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct BiomeMap(Vec<i32>);

impl BiomeMap {
    fn down(&self, id: i32) -> i32 {
        usize::try_from(id)
            .ok()
            .and_then(|i| self.0.get(i).copied())
            .unwrap_or(id)
    }
    /// Bits of the client's global biome palette (`Mth.ceillog2` of its biome count).
    fn global_bits(&self) -> u8 {
        let n = self.0.iter().copied().max().map_or(1, |m| m + 1).max(1) as u32;
        (32 - (n - 1).leading_zeros()) as u8
    }
    fn container(&self, p: &Paletted) -> Paletted {
        if !p.palette.is_empty() {
            let mut out = p.clone();
            out.palette.iter_mut().for_each(|b| *b = self.down(*b));
            return out;
        }
        let values: Vec<i32> = (0..chunk::BIOMES).map(|i| self.down(p.get(i))).collect();
        Paletted::from_values(&values, Strategy::Biomes, self.global_bits())
    }
}

/// `select_known_packs` to the client: the pinned core pack becomes the client's release.
pub(super) fn core_pack_down(version: ServerVersion, payload: &[u8]) -> Result<Vec<u8>> {
    swap_core(payload, crate::GAME_VERSION, version.name())
}

/// The client's `select_known_packs` reply: its release's core pack becomes the pinned one again.
pub(super) fn core_pack_up(version: ServerVersion, payload: &[u8]) -> Result<Vec<u8>> {
    swap_core(payload, version.name(), crate::GAME_VERSION)
}

fn swap_core(payload: &[u8], from: &str, to: &str) -> Result<Vec<u8>> {
    let mut p = configuration::SelectKnownPacks::read(&mut Reader::new(payload))?;
    for pack in &mut p.packs {
        if pack.namespace == "minecraft" && pack.id == "core" && pack.version == from {
            pack.version = to.into();
        }
    }
    let mut w = Writer::new();
    configuration::SelectKnownPacks::write(&p, &mut w);
    Ok(w.data)
}

/// Whether `payload` (a `registry_data`) is a synchronised registry `version` doesn't have at all: not
/// sent, whether or not the 26.2 layouts are on (`layout_26_2::NEW_REGISTRIES`, the same comparison).
pub(super) fn absent_registry(version: ServerVersion, payload: &[u8]) -> Result<bool> {
    if version != ServerVersion::V26_2 {
        return Ok(false);
    }
    let p = configuration::RegistryData::read(&mut Reader::new(payload))?;
    Ok(super::super::layout_26_2::NEW_REGISTRIES.contains(&p.registry.as_str()))
}

/// The biome registry without the entries `version` lacks, and the renumbering that follows from it.
/// `None`: not the biome registry, or nothing removed.
pub(super) fn registry_data(
    version: ServerVersion,
    payload: &[u8],
) -> Result<Option<(Vec<u8>, BiomeMap)>> {
    let removed = removed(version);
    if removed.is_empty() {
        return Ok(None);
    }
    let mut p = configuration::RegistryData::read(&mut Reader::new(payload))?;
    if p.registry != BIOMES
        || !p
            .entries
            .iter()
            .any(|e| removed.iter().any(|(r, _)| e.id == *r))
    {
        return Ok(None);
    }
    let kept: Vec<&str> = p
        .entries
        .iter()
        .map(|e| e.id.as_str())
        .filter(|id| !removed.iter().any(|(r, _)| id == r))
        .collect();
    let index = |name: &str| kept.iter().position(|k| *k == name).map(|i| i as i32);
    let map = p
        .entries
        .iter()
        .map(|e| match removed.iter().find(|(r, _)| e.id == *r) {
            // The stand-in, or the first biome if a data pack took the stand-in away.
            Some((_, instead)) => index(instead).unwrap_or(0),
            None => index(&e.id).unwrap_or(0),
        })
        .collect();
    p.entries
        .retain(|e| !removed.iter().any(|(r, _)| e.id == *r));
    let mut w = Writer::new();
    configuration::RegistryData::write(&p, &mut w);
    Ok(Some((w.data, BiomeMap(map))))
}

/// A column's sections with their biomes renumbered (pinned layout otherwise; `block_bits` is the pinned
/// global block state width).
pub(super) fn level_chunk(map: &BiomeMap, block_bits: u8, payload: &[u8]) -> Result<Vec<u8>> {
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
pub(super) fn chunks_biomes(map: &BiomeMap, payload: &[u8]) -> Result<Vec<u8>> {
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

/// The biome tags' entries renumbered (duplicates a stand-in makes are kept once).
pub(super) fn tags(map: &BiomeMap, registries: &mut [configuration::RegistryTags]) {
    for registry in registries.iter_mut().filter(|r| r.registry == BIOMES) {
        for tag in &mut registry.tags {
            let mut ids: Vec<i32> = Vec::with_capacity(tag.ids.len());
            for id in tag.ids.iter().map(|&id| map.down(id)) {
                if !ids.contains(&id) {
                    ids.push(id);
                }
            }
            tag.ids = ids;
        }
    }
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

    #[test]
    fn a_26_2_client_is_offered_its_own_core_pack_and_its_answer_goes_back_up() {
        let offer = SelectKnownPacks {
            packs: vec![KnownPack {
                namespace: "minecraft".into(),
                id: "core".into(),
                version: crate::GAME_VERSION.into(),
            }],
        };
        let down = core_pack_down(ServerVersion::V26_2, &encode(&offer)).unwrap();
        let seen = SelectKnownPacks::read(&mut Reader::new(&down)).unwrap();
        assert_eq!(seen.packs[0].version, "26.2");
        let up = core_pack_up(ServerVersion::V26_2, &down).unwrap();
        assert_eq!(
            SelectKnownPacks::read(&mut Reader::new(&up)).unwrap(),
            offer
        );
        assert!(own_core_pack(ServerVersion::V26_2));
        assert!(!own_core_pack(ServerVersion::Current));
    }

    fn biome_registry() -> Vec<u8> {
        let names = [
            "minecraft:badlands",
            "minecraft:dappled_forest",
            "minecraft:forest",
            "minecraft:the_void",
        ];
        encode(&configuration::RegistryData {
            registry: BIOMES.into(),
            entries: names
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
        let (out, map) = registry_data(ServerVersion::V26_2, &biome_registry())
            .unwrap()
            .unwrap();
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
        // badlands 0, dappled_forest -> forest (1), forest 1, the_void 2
        assert_eq!(map, BiomeMap(vec![0, 1, 1, 2]));
        assert!(
            registry_data(ServerVersion::Current, &biome_registry())
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn registries_26_2_does_not_synchronise_are_not_sent_to_it() {
        let reg = |name: &str| {
            encode(&configuration::RegistryData {
                registry: name.into(),
                entries: vec![],
            })
        };
        assert!(
            absent_registry(
                ServerVersion::V26_2,
                &reg("minecraft:decorated_pot_pattern")
            )
            .unwrap()
        );
        assert!(!absent_registry(ServerVersion::V26_2, &reg("minecraft:worldgen/biome")).unwrap());
        assert!(
            !absent_registry(
                ServerVersion::Current,
                &reg("minecraft:decorated_pot_pattern")
            )
            .unwrap()
        );
    }

    #[test]
    fn biome_containers_and_tags_take_the_clients_numbering() {
        let map = BiomeMap(vec![0, 1, 1, 2]);
        // a single-valued container of the_void (3 pinned) and a palette with all four
        let single = Paletted::from_values(&[3; chunk::BIOMES], Strategy::Biomes, 2);
        assert_eq!(map.container(&single).get(0), 2);
        let mixed: Vec<i32> = (0..chunk::BIOMES as i32).map(|i| i % 4).collect();
        let down = map.container(&Paletted::from_values(&mixed, Strategy::Biomes, 2));
        assert_eq!(
            (0..4).map(|i| down.get(i)).collect::<Vec<_>>(),
            [0, 1, 1, 2]
        );
        let mut tags = vec![configuration::RegistryTags {
            registry: BIOMES.into(),
            tags: vec![configuration::TagEntry {
                name: "minecraft:is_forest".into(),
                ids: vec![1, 2, 3],
            }],
        }];
        super::tags(&map, &mut tags);
        assert_eq!(tags[0].tags[0].ids, [1, 2]);
    }
}
