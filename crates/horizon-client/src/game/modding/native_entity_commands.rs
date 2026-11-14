//! Entity command execution shared by the bool and feedback-returning mod API methods.
use super::{Game, commands::execute, native_server::system_chat};
use horizon_client::text::Text;

impl Game<'_> {
    /// Runs the command `line` (a leading slash allowed) with the entity as
    /// its source, as vanilla runs a portal's or a sign's command:
    /// `ServerPlayer.createCommandSourceStack` (another entity:
    /// `Entity.createCommandSourceStackForNameResolution`), `.withPermission(level)`,
    /// then `Commands.performPrefixedCommand`. `level` is taken as given, above
    /// or below the entity's own. `@s` is the entity, and the command runs from
    /// its position and rotation in its level. A player is answered as for their
    /// own command (feedback and failures), and the other operators hear of it
    /// (`broadcastToAdmins`); another entity's source is silent
    /// (`CommandSource.NULL`). Whether the command ran: false for a line that
    /// fails to parse or to run, and, with nothing run, for a stale or removed
    /// entity, an entity outside the level in place (a player in another level
    /// included: that level is parked while the hook runs), and on a client.
    /// During a guest's turn, displaced outer players are excluded, as in
    /// `native_run_command`.
    pub(super) fn entity_run_command(
        &mut self,
        entity: mod_api::EntityRef,
        line: &str,
        level: mod_api::PermissionLevel,
    ) -> bool {
        self.run_entity_command(entity, line, level).0
    }

    /// The entity command's outcome and source feedback, also delivered to the player.
    pub(super) fn entity_run_command_with_output(
        &mut self,
        entity: mod_api::EntityRef,
        line: &str,
        level: mod_api::PermissionLevel,
    ) -> mod_api::CommandResult {
        let (success, lines) = self.run_entity_command(entity, line, level);
        mod_api::CommandResult::new(success, lines.iter().map(Text::plain).collect())
    }

    fn run_entity_command(
        &mut self,
        entity: mod_api::EntityRef,
        line: &str,
        level: mod_api::PermissionLevel,
    ) -> (bool, Vec<Text>) {
        use super::commands::Target;
        if self.lan.is_none() {
            return (false, Vec::new());
        }
        let Some(target) = self.native_entity_target(entity) else {
            return (false, Vec::new());
        };
        let line = line.strip_prefix('/').unwrap_or(line);
        let permission = level.level();
        match target {
            // The acting player, in place for this turn.
            Target::Player => match self.lan_acting_uuid {
                Some(uuid) => self.run_command_as_player(uuid, line, permission),
                None => (false, Vec::new()),
            },
            Target::Guest(conn) => {
                let Some(host) = &self.lan else {
                    return (false, Vec::new());
                };
                let Some(uuid) = host
                    .hub
                    .guest(conn)
                    .filter(|g| host.hub.here(g))
                    .map(|g| g.uuid)
                else {
                    return (false, Vec::new());
                };
                self.as_guest(conn, |game| {
                    game.run_command_as_player(uuid, line, permission)
                })
                .unwrap_or_default()
            }
            target => {
                let Some(pos) = self.target_position(target) else {
                    return (false, Vec::new());
                };
                // `CommandSource.NULL`: no feedback, no failure, no operator told.
                let source = execute::Source {
                    entity: Some(target),
                    pos,
                    rot: self.target_rotation(target),
                    eyes: false,
                    silent: true,
                    store: None,
                    dimension: self.dim,
                    origin_dimension: self.dim,
                    origin_player: false,
                    level: None,
                    block: None,
                    cart: None,
                };
                self.run_command_from(source, line, permission)
            }
        }
    }
    /// `entity_run_command` for the player `uuid` in place: their own source,
    /// its lines sent to them and its `broadcastToAdmins` to the other operators.
    fn run_command_as_player(
        &mut self,
        uuid: u128,
        line: &str,
        permission: u8,
    ) -> (bool, Vec<Text>) {
        use super::commands::Target;
        // `ServerPlayer.createCommandSourceStack`: the player's own position and
        // rotation, never an outer command's (`player_source` reads those).
        let Some(pos) = self.target_position(Target::Player) else {
            return (false, Vec::new());
        };
        let source = execute::Source {
            pos,
            rot: self.target_rotation(Target::Player),
            ..self.player_source()
        };
        let outer_admins = std::mem::take(&mut self.admin_feedback);
        let (ok, lines) = self.run_command_from(source, line, permission);
        for line in &lines {
            self.native_send(uuid, system_chat(line));
        }
        self.send_admin_feedback(self.lan_acting);
        self.admin_feedback = outer_admins;
        (ok, lines)
    }
    /// `line` run from `source` at `permission`, with what it tells its source:
    /// the outer command's source, state and output are kept apart and put back.
    fn run_command_from(
        &mut self,
        source: execute::Source,
        line: &str,
        permission: u8,
    ) -> (bool, Vec<Text>) {
        let outer = self.command_source.replace(source);
        let filter = std::mem::replace(&mut self.native_command_filter, true);
        let console = self.console_broadcast.take();
        let capture = self.chat_capture.replace(Vec::new());
        let outer_exec = std::mem::take(&mut self.exec);
        let level = std::mem::replace(&mut self.ui.registries.permission, permission);
        let ok = self.run_command(line);
        self.ui.registries.permission = level;
        self.exec = outer_exec;
        let lines = std::mem::replace(&mut self.chat_capture, capture).unwrap_or_default();
        self.console_broadcast = console;
        self.native_command_filter = filter;
        self.command_source = outer;
        (ok, lines)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::native_guest_turn_tests::with_guests;
    use horizon_protocol::packets::play::Clientbound as C;
    use mod_api::{GameMode, PermissionLevel};

    #[test]
    #[ignore = "daily"]
    fn player_command_returns_permission_failures_and_success_without_swallowing_chat() {
        with_guests(|game| {
            game.build_registries();
            let player = game.player(12).unwrap();
            assert_eq!(player.game_mode(game), Some(GameMode::Survival));
            for permission in [0, 2] {
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(12, |guest| guest.permission = permission)
                    .unwrap();
                game.as_guest(12, |game| {
                    let level = PermissionLevel::from_level(i32::from(
                        player.permission_level(game).unwrap(),
                    ));
                    let before = game.guest_packets.len();
                    // A surrounding command's output must stay separate from this run.
                    game.chat_capture = Some(vec![Text::literal("outer command")]);
                    let result =
                        player
                            .entity()
                            .run_command_with_output(game, "/gamemode creative", level);
                    assert_eq!(result.success, permission == 2);
                    if permission == 0 {
                        assert_eq!(
                            result.feedback,
                            [
                                "Unknown or incomplete command. See below for error",
                                "gamemode creative<--[HERE]",
                            ]
                        );
                        assert_eq!(player.game_mode(game), Some(GameMode::Survival));
                    } else {
                        assert_eq!(result.feedback, ["Set own game mode to Creative Mode"]);
                        assert_eq!(player.game_mode(game), Some(GameMode::Creative));
                    }
                    let chat: Vec<_> = game.guest_packets[before..]
                        .iter()
                        .filter_map(|packet| match packet {
                            C::SystemChat(chat) if !chat.overlay => {
                                Some(horizon_client::component::name_text(&chat.content).plain())
                            }
                            _ => None,
                        })
                        .collect();
                    assert_eq!(chat, result.feedback);
                    let outer = game.chat_capture.take().unwrap();
                    assert_eq!(outer.len(), 1);
                    assert_eq!(outer[0].plain(), "outer command");
                })
                .unwrap();
            }
            assert_eq!(player.game_mode(game), Some(GameMode::Creative));
        });
    }
}
