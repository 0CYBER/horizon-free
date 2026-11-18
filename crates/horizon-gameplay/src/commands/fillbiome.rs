//! `/fillbiome`.
use super::host::{CommandHost, R};
use horizon_client::{
    command::{Arg, Context},
    text::Text,
};

/// What `/fillbiome` needs from the game: the world's biome cells and its position arguments.
pub trait FillbiomeHost: CommandHost {
    /// `BlockPosArgument.getLoadedBlockPos`.
    fn block_pos_arg(&self, ctx: &Context, name: &str) -> R<[i32; 3]>;
    /// The `max_block_modifications` game rule.
    fn max_block_modifications(&self) -> i32;
    /// The index of the biome named `name` in the world's biome table.
    fn biome_index(&self, name: &str) -> Option<usize>;
    /// Whether the column `key` is loaded.
    fn column_loaded(&self, key: (i32, i32)) -> bool;
    /// The biome index of the cell holding `(x, y, z)`; none when its column is not loaded.
    fn biome_cell_at(&self, x: i32, y: i32, z: i32) -> Option<u16>;
    /// Whether the biome `current` (an index) matches the `filter` argument.
    fn biome_matches_filter(&self, filter: &str, current: u16) -> bool;
    /// Sets the biome cell holding `(x, y, z)` to the biome index `target`.
    fn set_biome_cell_at(&mut self, x: i32, y: i32, z: i32, target: usize);
    /// The tail of a biome change: save, re-mesh and resend the `touched` columns.
    fn commit_biome_columns(
        &mut self,
        touched: &std::collections::HashSet<(i32, i32)>,
        columns: &[(i32, i32)],
    );
}

fn num(n: impl std::fmt::Display) -> Text {
    Text::literal(n.to_string())
}

/// `/fillbiome <from> <to> <biome> [replace <filter>]`.
pub fn fillbiome(host: &mut dyn FillbiomeHost, ctx: &Context) -> R<i32> {
    let Some(Arg::Resource(biome)) = ctx.get("biome").cloned() else {
        return Ok(0);
    };
    // BlockPosArgument.getLoadedBlockPos on both corners first.
    let (a, b) = (
        host.block_pos_arg(ctx, "from")?,
        host.block_pos_arg(ctx, "to")?,
    );
    // Quantize to biome cells (`QuartPos`), then work with the box.
    let q = |v: i32| (v >> 2) << 2;
    let lo = [
        q(a[0]).min(q(b[0])),
        q(a[1]).min(q(b[1])),
        q(a[2]).min(q(b[2])),
    ];
    let hi = [
        q(a[0]).max(q(b[0])),
        q(a[1]).max(q(b[1])),
        q(a[2]).max(q(b[2])),
    ];
    let volume =
        i64::from(hi[0] - lo[0] + 1) * i64::from(hi[1] - lo[1] + 1) * i64::from(hi[2] - lo[2] + 1);
    let limit = host.max_block_modifications();
    if volume > i64::from(limit) {
        return Err(host.error("commands.fillbiome.toobig", &[&limit, &volume]));
    }
    let Some(target) = host.biome_index(&biome) else {
        return Err(host.error(
            "argument.resource.not_found",
            &[&biome, &"minecraft:worldgen/biome"],
        ));
    };
    let filter = match ctx.get("filter") {
        Some(Arg::Resource(f)) => Some(f.clone()),
        _ => None,
    };
    for cz in (lo[2] >> 4)..=(hi[2] >> 4) {
        for cx in (lo[0] >> 4)..=(hi[0] >> 4) {
            if !host.column_loaded((cx, cz)) {
                return Err(host.error("argument.pos.unloaded", &[]));
            }
        }
    }
    let mut changed = 0;
    let mut touched = std::collections::HashSet::new();
    for z in (lo[2]..=hi[2]).step_by(4) {
        for y in (lo[1]..=hi[1]).step_by(4) {
            for x in (lo[0]..=hi[0]).step_by(4) {
                let key = (x >> 4, z >> 4);
                let Some(current) = host.biome_cell_at(x, y, z) else {
                    continue;
                };
                if let Some(f) = &filter
                    && !host.biome_matches_filter(f, current)
                {
                    continue;
                }
                if usize::from(current) == target {
                    continue;
                }
                host.set_biome_cell_at(x, y, z, target);
                touched.insert(key);
                changed += 1;
            }
        }
    }
    if changed == 0 {
        return Err(host.error("commands.fillbiome.no_changes", &[]));
    }
    let columns: Vec<(i32, i32)> = ((lo[2] >> 4)..=(hi[2] >> 4))
        .flat_map(|cz| ((lo[0] >> 4)..=(hi[0] >> 4)).map(move |cx| (cx, cz)))
        .filter(|key| touched.contains(key))
        .collect();
    host.commit_biome_columns(&touched, &columns);
    host.feedback(Text::translate(
        "commands.fillbiome.success.count",
        &[
            num(changed),
            num(lo[0]),
            num(lo[1]),
            num(lo[2]),
            num(hi[0]),
            num(hi[1]),
            num(hi[2]),
        ],
    ));
    Ok(changed)
}
