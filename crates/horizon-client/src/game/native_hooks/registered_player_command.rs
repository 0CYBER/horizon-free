use super::*;

impl Game<'_> {
    pub(in crate::game) fn native_registered_player_command(
        &mut self,
        player: PlayerRef,
        permission: u8,
        raw: &str,
        ran: &mut bool,
    ) {
        self.fire_registered_player_command(player, permission, raw, ran);
    }
}
