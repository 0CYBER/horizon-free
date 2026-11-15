fn head(game: &mut Game<'_>) {
    fnr_registry::note("HEAD on_tick_rate_changed, ticks_until_autosave", game.ticks_until_autosave);
}
