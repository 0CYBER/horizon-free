pub trait Notes {
    #[notes = r#"## Bone meal at a position

`LevelRef::bone_meal(host, pos)` runs the whole bone meal sequence on one block for a mod that
grows things itself (a sprinkler, a growth totem), so it neither reimplements the checks nor
skips `bonemeal` listeners. It takes no stack: nothing is consumed, and a hook that edits the
stack it is shown edits a throwaway one."#]
    fn level_block_writes_note0();
    #[notes = r#"## Setting a biome

`LevelRef::set_biome(host, pos, biome)` changes one biome cell, the 4x4x4 block volume a biome is
stored in, so every position in that cell changes with it. It shares `/fillbiome`'s save, mesh and
resend path, so the change survives a reload and reaches players as the command's does. Name a
biome the world knows (a data pack's included); anything else is refused.
Tentative reads see the new cells inside a transaction. Abort restores the cells without
publishing. Saving, remeshing and resending wait for the outermost commit, once per column;
outside a transaction they run immediately."#]
    fn level_block_writes_note1();
}
