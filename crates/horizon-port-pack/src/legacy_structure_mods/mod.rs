//! The conversion of three families of structure mods' data, kept apart from
//! the general conversion code (`worldgen`, `processors`, `carry`):
//!
//! - [`yung`]: YUNG's API and the mods built on it (Better Dungeons, Desert
//!   Temples, Jungle Temples, Witch Huts, Ocean Monuments, Strongholds,
//!   Fortresses, Mineshafts), written under `horizon:yungsapi` and the other
//!   keys `horizon_world::worldgen::structure::yung` reads.
//! - [`lithostitched`]: Lithostitched's jigsaw, worldgen modifiers and
//!   processors.
//! - [`ctov`]: Choice Theorem's Overhauled Village's code-registered set
//!   entries and its modular compat processor.
//!
//! The general code reaches them only through the `pub(crate)` entry points
//! of these three modules. Nothing in them is used by another mod's
//! conversion.
#[allow(dead_code)] // its modular compat is `crate::lithostitched`'s now; the rest goes with C1
pub(crate) mod ctov;

#[cfg(test)]
#[allow(dead_code)] // the old Lithostitched tests used it
fn data() -> crate::processors::ProcessorData {
    let mut d = crate::processors::ProcessorData::default();
    d.spawners.insert(
        "repurposed_structures:dungeons/badlands".into(),
        vec![
            ("minecraft:husk".into(), 10.),
            ("minecraft:cave_spider".into(), 3.),
        ],
    );
    d.replace_vanilla.insert(("mvs".into(), "village".into()));
    d
}
