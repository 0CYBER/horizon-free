//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The server host's methods of the `world_edit` area, implemented by the game and reached through [`ServerHost`].
#[horizon_mod_api_macros::host_area]
pub trait WorldEditServerHost {
    /// Side: server.
    /// For a server mod: the runs made at once, straight into the chunks of the authoritative level `dimension`
    /// (the one being ticked, as for `native_set_block`), with what `fill ... strict` does for block entities (a
    /// block replaced by another loses its block entity, and a new block gets the one `native_set_block` would
    /// give it, a mod block's included); no neighbour is told, and the chunks changed go to the players whole.
    /// Runs in chunks not loaded, and the parts of runs outside the column, are left out. The blocks changed, or
    /// why the edit was left out, with nothing changed: another dimension, a client of a server, a palette entry
    /// or predicate that fails typed validation, a run naming a palette entry or filter past the end,
    /// or more than 65536 filters.
    #[access(WorldOnly)]
    fn edit_blocks(&mut self, dimension: &str, edit: &BlockEdit) -> Result<u64, String>;
    /// Side: server.
    /// A block entity's data saved at `data_version` (a schematic's, a structure file's, an older world's), upgraded
    /// as the chunk loader upgrades a block entity of a chunk of that version: its type's datafixes up to the game's
    /// DataVersion. Data of exactly the game's version comes back unchanged, and so does a block entity whose `id`
    /// that version doesn't know, or data the fixes can't read. The chunk-wide fixes (blocks around it, bed block
    /// entities removed) need a chunk and don't run. Pure: nothing in the world changes. `Err` names the refusal.
    /// Before any fix runs: a negative version ([`NbtUpgradeError::NegativeVersion`]), one newer than the game's
    /// ([`NbtUpgradeError::FutureVersion`]), or data with a list or compound 512 levels deep or more, counted as
    /// encoded with the wrapper compound of each mixed-list element as a level ([`NbtUpgradeError::TooDeep`], the
    /// depth cap of every NBT read). After the fixes: upgraded data that deep ([`NbtUpgradeError::UpgradedTooDeep`]),
    /// as no NBT read could take it back. There is no size refusal.
    #[access(WorldOnly)]
    fn upgrade_block_entity_nbt(
        &self,
        data_version: i32,
        nbt: Compound,
    ) -> Result<Compound, NbtUpgradeError>;
    /// Side: server.
    /// An entity's data saved at `data_version`, upgraded as the entity chunk loader upgrades an entity of a chunk
    /// of that version: its type's datafixes, its passengers' included, up to the game's DataVersion. Data of exactly
    /// the game's version, or data the fixes can't read, comes back unchanged. Pure: nothing in the world changes.
    /// Refused by name as `upgrade_block_entity_nbt` refuses.
    #[access(WorldOnly)]
    fn upgrade_entity_nbt(
        &self,
        data_version: i32,
        nbt: Compound,
    ) -> Result<Compound, NbtUpgradeError>;
    /// Side: server.
    /// For a server mod: each column's height by `heightmap` in the authoritative level `dimension` (the one being
    /// ticked), none for a column not loaded. `Err` says why nothing was read: another dimension or a client of a
    /// server, or more than 65,536 columns (`MAX_COLUMNS`).
    #[access(WorldOnly)]
    fn surface_heights(
        &self,
        dimension: &str,
        columns: &[(i32, i32)],
        heightmap: Heightmap,
    ) -> Result<Vec<Option<i32>>, String>;
    /// Side: server.
    /// For a server mod: `below` blocks under and `above` blocks from each column's height by `heightmap`, as a
    /// palette of complete `BlockState` values and indexes, with biome tints (air outside the column and in columns
    /// not loaded). The tints are the biome's at the column's top, not blended over the biome blend radius as the
    /// game draws them: near a biome border they can differ from what players see. `Err` says why nothing was
    /// read: another dimension or a client of a server, `below + above` of 0 or more than the level's height, or
    /// a read of more than 1,048,576 blocks (`MAX_READ`; it runs on the server tick, so read big areas in pieces).
    #[access(WorldOnly)]
    fn read_columns(
        &self,
        dimension: &str,
        columns: &[(i32, i32)],
        heightmap: Heightmap,
        range: (u32, u32),
    ) -> Result<ColumnBlocks, String>;
    /// Side: server.
    /// For a server mod: structure template `id` (`mymod:hut`, found as
    /// `StructureTemplateManager.get` finds it: the world's saved structures,
    /// then the game's and the mods' own) as `/place template id` would place
    /// it at `at` with `rotation` and `mirror`, without placing anything. The
    /// blocks come in vanilla's placement order (full blocks, the others, then
    /// those with a block entity, each by y, x, z), from the palette
    /// `/place template` picks at `at`; no processor runs, and the template's
    /// entities are left out. None for an id that doesn't parse or has a `.`
    /// or `..` part, a template the game doesn't have, and on a client.
    /// API @API:structure_template@.
    #[access(WorldOnly)]
    fn structure_template(
        &mut self,
        id: &str,
        at: [i32; 3],
        rotation: StructureRotation,
        mirror: StructureMirror,
    ) -> Option<StructureTemplateSnapshot>;
}
