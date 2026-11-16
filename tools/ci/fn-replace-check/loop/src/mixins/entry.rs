fn entry(game: &mut Game<'_>) {
    let time = game.world.game_time;
    if time % 100 == 0 {
        fnr_loop::note(&format!("HEAD tick_block_entities at game time {time}"));
    }
}
