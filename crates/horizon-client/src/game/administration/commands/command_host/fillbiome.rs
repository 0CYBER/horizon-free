use crate::game::Game;
use horizon_client::command::Context;
use horizon_gameplay::commands::{fillbiome::FillbiomeHost, host::R};

impl FillbiomeHost for Game<'_> {
    fn block_pos_arg(&self, ctx: &Context, name: &str) -> R<[i32; 3]> {
        Game::block_pos_arg(self, ctx, name)
    }
    fn max_block_modifications(&self) -> i32 {
        self.ui
            .rules
            .int_at(horizon_client::gamerules::vanilla::MAX_BLOCK_MODIFICATIONS)
    }
    fn biome_index(&self, name: &str) -> Option<usize> {
        world_biomes!(self).list.iter().position(|b| b.name == name)
    }
    fn column_loaded(&self, key: (i32, i32)) -> bool {
        self.world.chunks.contains_key(&key)
    }
    fn biome_cell_at(&self, x: i32, y: i32, z: i32) -> Option<u16> {
        let chunk = self.world.chunks.get(&(x >> 4, z >> 4))?;
        Some(chunk.biome(x, y, z))
    }
    fn biome_matches_filter(&self, filter: &str, current: u16) -> bool {
        let current_name = world_biomes!(self).name(current);
        self.biome_matches(filter, current_name)
    }
    fn set_biome_cell_at(&mut self, x: i32, y: i32, z: i32, target: usize) {
        let key = (x >> 4, z >> 4);
        let qy =
            ((y - self.world.bottom_y()) >> 2).clamp(0, self.world.layout().height() / 4 - 1);
        self.world.chunks.mutate_chunk(key, |chunk| {
            let chunk = std::sync::Arc::make_mut(chunk);
            chunk.set_biome_cell((x & 15) >> 2, qy, (z & 15) >> 2, target as u16);
        });
    }
    fn commit_biome_columns(
        &mut self,
        touched: &std::collections::HashSet<(i32, i32)>,
        columns: &[(i32, i32)],
    ) {
        Game::commit_biome_columns(self, touched, columns);
    }
}
