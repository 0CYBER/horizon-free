//! Fixes of 1.8 to 1.12 (keys 100 to 1125, DataVersion up to 1343): what worlds
//! saved by those versions need before the flattening ([`super::fixes_flat`]).
//! The options fixes of this range (505, 816, 1344) are client settings.
use super::dynamic::{as_int, as_int_stream, set, string};
use super::mark_read;
use super::schema::{Key, Ty, key};
use super::{Cx, FixFn, reread_as};
use crate::nbt::Tag;

pub(super) fn register(out: &mut Vec<(Key, FixFn)>) {
    out.push((key(1022, 0), |cx| {
        let k = cx.k;
        cx.each(Ty::Player, |p| reread_as(Ty::Player, k, p))
    }));
    out.push((key(1125, 0), bed_block_entities));
    out.push((key(1125, 0), |cx| {
        cx.item(
            |id| id == "minecraft:bed",
            |s| {
                if as_int(s.get("Damage"), 0) == 0 {
                    set(s, "Damage", Tag::Short(14));
                }
            },
        )
    }));
}

// ------------------------------------------------------------------ 1125

/// `ChunkBedBlockEntityInjecterFix`: a bed block gets the block entity 1.12 gave it.
fn bed_block_entities(cx: &mut Cx) {
    cx.each(Ty::Chunk, |chunk| {
        let Some(level) = chunk.get_mut("Level") else {
            return;
        };
        let chunk_x = as_int(level.get("xPos"), 0);
        let chunk_z = as_int(level.get("zPos"), 0);
        let mut beds = Vec::new();
        if let Some(Tag::List(sections)) = level.get("Sections") {
            for section in sections {
                let y = as_int(section.get("Y"), 0);
                for (p, block) in as_int_stream(section.get("Blocks"))
                    .unwrap_or_default()
                    .into_iter()
                    .enumerate()
                {
                    if (block & 0xFF) << 4 == 416 {
                        let (x, yy, z) = (p as i32 & 15, p as i32 >> 8 & 15, p as i32 >> 4 & 15);
                        let mut bed = Tag::compound([
                            ("id", string("minecraft:bed")),
                            ("x", Tag::Int(x + (chunk_x << 4))),
                            ("y", Tag::Int(yy + (y << 4))),
                            ("z", Tag::Int(z + (chunk_z << 4))),
                            ("color", Tag::Short(14)),
                        ]);
                        mark_read(&mut bed);
                        beds.push(bed);
                    }
                }
            }
        }
        if beds.is_empty() {
            return;
        }
        if !matches!(level.get("TileEntities"), Some(Tag::List(_))) {
            set(level, "TileEntities", Tag::List(Vec::new()));
        }
        if let Some(Tag::List(entities)) = level.get_mut("TileEntities") {
            entities.extend(beds);
        }
    });
}
