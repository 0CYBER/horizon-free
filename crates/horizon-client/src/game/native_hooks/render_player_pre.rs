use super::*;

impl Game<'_> {
    /// Before a player is drawn (NeoForge's `RenderPlayerEvent.Pre`): once a frame for each other
    /// player in view and this window's player in third person, before its `render_living_pre`.
    /// `canceled` skips all of the player's drawing this frame and the other three hooks of the
    /// pair; uncanceled, `hidden` names the vanilla model's parts to leave out.
    pub(in crate::game) fn native_render_player_pre(
        &self,
        player: &MobRender<'_>,
        who: &PlayerRender<'_>,
        draw: &mut BlockEntityDraw,
        hidden: &mut HiddenParts,
        canceled: &mut bool,
    ) {
        self.fire_render_player_pre(player, who, draw, hidden, canceled);
    }
}
