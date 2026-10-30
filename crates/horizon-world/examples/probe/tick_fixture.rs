//! Process vanilla saved block/fluid ticks through Horizon's storage and queues.
use horizon_core::{
    position::ChunkPos,
    ticks::{ScheduledTick, TickPriority, TickTypeId},
};
use horizon_world::{
    chunk::ChunkData,
    compression::Compression,
    nbt::{self, Limits, Tag},
    region::{ChunkRecord, RegionFile, RegionWriter},
    tick_data::TickKind,
};
use std::{collections::BTreeMap, error::Error, fs, path::PathBuf};
pub fn main() -> Result<(), Box<dyn Error>> {
    let base = PathBuf::from(crate::args().nth(1).ok_or("expected fixture directory")?);
    let bytes = fs::read(base.join("ticks-input.nbt"))?;
    let (root, used) = nbt::read_named(&bytes, Limits::default())?;
    assert_eq!(used, bytes.len());
    let mut chunk = ChunkData::new(root)?;
    for (kind, ids_field, time) in [
        (TickKind::Block, "block_ids", 1000),
        (TickKind::Fluid, "fluid_ids", i64::MAX - 8),
    ] {
        let Some(Tag::Compound(entries)) = chunk.root().tag.get(ids_field) else {
            return Err("missing type IDs".into());
        };
        let mut ids = BTreeMap::new();
        let mut names = BTreeMap::new();
        for (name, id) in entries {
            let Tag::Int(id) = id else {
                return Err("invalid ID".into());
            };
            let name = name.to_string()?;
            ids.insert(name.clone(), TickTypeId(*id as u32));
            names.insert(TickTypeId(*id as u32), name);
        }
        let mut ticks = chunk.saved_ticks(kind, |name| ids.get(name).copied())?;
        assert!(ticks.peek().is_none());
        assert_eq!(ticks.count(), 251);
        ticks.unpack(time);
        if let Some(first) = ticks.poll() {
            assert!(ticks.schedule(ScheduledTick {
                trigger_tick: time.wrapping_add(123),
                priority: TickPriority::High,
                sub_tick_order: 77,
                ..first
            }));
        }
        let saved = ticks.pack(time);
        chunk.set_saved_ticks(kind, &saved, |id| names.get(&id).cloned())?;
    }
    let root = chunk.into_nbt();
    fs::write(
        base.join("ticks-output.nbt"),
        nbt::write_named(&root, Limits::default())?,
    )?;
    let path = base.join("tick-region/r.0.0.mca");
    let record = ChunkRecord::from_nbt(&root, Compression::Zlib, 123, Limits::default())?;
    let mut writer = RegionWriter::create(&path)?;
    writer.write(ChunkPos::new(0, 0), &record)?;
    writer.finish()?;
    let record = RegionFile::open(path)?
        .read(ChunkPos::new(0, 0))?
        .ok_or("missing region chunk")?;
    assert_eq!(record.nbt(Limits::default())?, root);
    Ok(())
}
