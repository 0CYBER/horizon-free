fn before(game: &Game<'_>) {
    fnr_beta::note(&format!("BEFORE_CALL next_autosave_interval, rate {}", game.clock.rate()));
}
