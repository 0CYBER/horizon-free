fn head(game: &mut Game<'_>) -> ::core::ops::ControlFlow<()> {
    let rate = game.clock.rate();
    fnr_alpha::note(&format!(
        "HEAD on_tick_rate_changed: rate {rate}, ticks_until_autosave was {}",
        game.ticks_until_autosave
    ));
    if rate == 13.0 {
        fnr_alpha::note("HEAD cancels on_tick_rate_changed at rate 13");
        return ::core::ops::ControlFlow::Break(());
    }
    game.ticks_until_autosave = 4242;
    ::core::ops::ControlFlow::Continue(())
}
