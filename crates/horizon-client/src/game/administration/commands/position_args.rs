//! Block position argument validation for command execution.
use super::{Context, Dim, Game, R, err};

impl Game<'_> {
    /// BlockPosArgument.getLoadedBlockPos.
    pub(in crate::game) fn block_pos_arg(&self, ctx: &Context, name: &str) -> R<[i32; 3]> {
        let c = self
            .coords(ctx, name)
            .ok_or_else(|| err("argument.pos.missing.int", &[]))?;
        // BlockPos.containing(getPosition)
        let p = self.resolve_coords(&c).map(|v| v.floor() as i32);
        let dimension = self.command_source.map_or(self.dim, |s| s.dimension);
        if dimension != self.dim || !self.world.chunks.contains_key(&(p[0] >> 4, p[2] >> 4)) {
            return Err(err("argument.pos.unloaded", &[]));
        }
        if p[1] < self.world.bottom_y() || p[1] >= self.world.top_y() {
            return Err(err("argument.pos.outofworld", &[]));
        }
        Ok(p)
    }
    /// `CloneCommands.clone`'s two `Level.hasChunksAt` calls, each in its own
    /// box's level: every chunk column between the corners must be loaded, and
    /// the box only has to overlap the build height (`LevelReader.hasChunksAt`,
    /// over `Level.getMinY()`/`getMaxY()`, not the chunks' wider column). The
    /// corners are walked in the order given, as vanilla's loop is.
    pub(in crate::game) fn clone_boxes_loaded(
        &mut self,
        boxes: [(Dim, [i32; 3], [i32; 3]); 2],
    ) -> R<()> {
        for (dim, a, b) in boxes {
            self.clone_enter(dim)?;
            let (min_y, max_y) = (self.world.shape.min_y, self.world.shape.max_y());
            if !(b[1] >= min_y
                && a[1] < max_y
                && (a[0] >> 4..=b[0] >> 4).all(|x| {
                    (a[2] >> 4..=b[2] >> 4).all(|z| self.world.chunks.contains_key(&(x, z)))
                }))
            {
                return Err(err("argument.pos.unloaded", &[]));
            }
        }
        Ok(())
    }
    /// `Level.setBlock`'s gate (`isInValidBounds`, the build height and not the
    /// chunks' wider column, and the chunk is resident): a write it refuses
    /// changes nothing, so `CloneCommands.clone` does not count it.
    pub(in crate::game) fn clone_writable(&self, pos: [i32; 3]) -> bool {
        pos[1] >= self.world.shape.min_y
            && pos[1] < self.world.shape.max_y()
            && self.world.chunks.contains_key(&(pos[0] >> 4, pos[2] >> 4))
    }
    /// BlockPosArgument.getSpawnablePos: no loaded-chunk or build-height check,
    /// only `Level.isInSpawnableBounds` (upper bounds exclusive).
    pub(in crate::game) fn spawnable_pos_arg(&self, ctx: &Context, name: &str) -> R<[i32; 3]> {
        let c = self
            .coords(ctx, name)
            .ok_or_else(|| err("argument.pos.missing.int", &[]))?;
        let p = self.resolve_coords(&c).map(|v| v.floor() as i32);
        if !(-30_000_000..30_000_000).contains(&p[0])
            || !(-30_000_000..30_000_000).contains(&p[2])
            || !(-20_000_000..20_000_000).contains(&p[1])
        {
            return Err(err("argument.pos.outofbounds", &[]));
        }
        Ok(p)
    }
}
