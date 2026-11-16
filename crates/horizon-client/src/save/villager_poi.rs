//! POI persistence reads the same immutable profession snapshot as job acquisition.
use super::*;
/// The chunk's points of interest as `PoiManager` saves them: each section
/// with POI blocks, valid, with a record per block at its max tickets (a
/// freshly made world). None when the chunk has none.
pub fn encode_poi(
    chunk: &Chunk,
    tables: &SaveTables,
    professions: Option<&crate::mobs::villager::ResolvedProfessions>,
) -> Option<NamedTag> {
    let kind = |v: u32| {
        tables.poi.get(v as usize).copied().flatten().or_else(|| {
            let table = professions?;
            let poi = table.workstation_pois.get(v as usize).copied().flatten()?;
            Some((table.pois.name(poi)?, poi.max_tickets() as i32))
        })
    };
    let mut sections = Compound::new();
    let mut states = [0u32; 4096];
    for (si, section) in chunk.sections.iter().enumerate() {
        if !section.blocks.maybe_has(|v| kind(v).is_some()) {
            continue;
        }
        section.unpack(&mut states);
        let y = (chunk.min_y() >> 4) + si as i32;
        let records: Vec<Tag> = states
            .iter()
            .enumerate()
            .filter_map(|(i, &v)| {
                let (id, tickets) = kind(v)?;
                let i = i as i32;
                let pos = [
                    chunk.x * 16 + (i & 15),
                    y * 16 + (i >> 8),
                    chunk.z * 16 + ((i >> 4) & 15),
                ];
                let mut record = vec![("pos", Tag::IntArray(pos.to_vec())), ("type", string(id))];
                // PoiRecord.CODEC: free_tickets defaults to 0 and is then left out.
                if tickets != 0 {
                    record.push(("free_tickets", Tag::Int(tickets)));
                }
                Some(Tag::compound(record))
            })
            .collect();
        if !records.is_empty() {
            sections.insert(
                y.to_string().as_str().into(),
                Tag::compound([("Valid", Tag::Byte(1)), ("Records", Tag::List(records))]),
            );
        }
    }
    (!sections.is_empty()).then(|| poi_root(sections))
}

impl SaveDir {
    /// Publish the owning world's immutable profession snapshot for subsequent queued writes.
    pub fn install_profession_snapshot(
        &self,
        table: Arc<crate::mobs::villager::ResolvedProfessions>,
    ) {
        *self
            .professions
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(table);
    }
}
