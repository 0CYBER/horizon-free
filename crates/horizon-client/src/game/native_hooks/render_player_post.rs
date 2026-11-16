use super::*;

impl Game<'_> {
    /// After a player is drawn (NeoForge's `RenderPlayerEvent.Post`): once a frame for each player
    /// whose `render_player_pre` wasn't canceled, after its `render_living_post` (right after its
    /// `render_living_pre` when only that one was canceled, with no parts then).
    pub(in crate::game) fn native_render_player_post(
        &self,
        player: &MobRender<'_>,
        who: &PlayerRender<'_>,
        parts: Option<&PosedParts>,
        draw: &mut BlockEntityDraw,
    ) {
        self.fire_render_player_post(player, who, parts, draw);
    }
}
