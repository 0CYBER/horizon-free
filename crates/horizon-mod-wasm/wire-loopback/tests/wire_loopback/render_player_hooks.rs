pub(super) struct RenderPlayerHooks;
impl RenderPlayerHooks {
    pub(super) fn render_player_pre(
        &self,
        _: &dyn mod_api::ClientHost,
        event: &mut mod_api::RenderPlayerPreEvent<'_>,
    ) -> mod_api::Flow {
        // This window's own player is drawn by the mod: every other one isn't.
        assert_eq!(event.player.kind, "minecraft:player");
        event.cancelled = event.who.local;
        event.draw.set_light(event.who.name.len() as u32);
        mod_api::Flow::Continue
    }
    pub(super) fn render_player_post(
        &self,
        _: &dyn mod_api::ClientHost,
        event: &mut mod_api::RenderPlayerPostEvent<'_>,
    ) {
        assert_eq!((event.who.name, event.who.local), ("test:player", true));
        event.draw.set_light(42);
    }
}
