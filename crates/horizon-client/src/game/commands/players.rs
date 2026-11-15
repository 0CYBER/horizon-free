//! Commands acting on the other players of a hosted world or a server
//! (EntitySelector.findPlayers and EntityArgument's player targets). The
//! player in place (the host's, or the guest whose turn it is) is
//! `Target::Player`; every other guest is `Target::Guest`, found by name,
//! UUID or selector, and what a command does to them runs with their state
//! swapped in (`for_player`), as their own turn, what it tells them going to
//! their chat.
use super::{Game, Target};
use horizon_client::{command::args::Selector, lan::guest::Guest, text::Text};
use horizon_protocol::packets::play::{self, Clientbound as C};

impl Target {
    /// A player, in place or not (`instanceof ServerPlayer`).
    pub(in crate::game) fn is_player(self) -> bool {
        matches!(self, Target::Player | Target::Guest(_))
    }
}

/// The target of a command whose target is optional and left out. `CommandSourceStack.getEntityOrException`
/// (`player_only` false: `kill`, `effect clear`, `swing`) takes the source's entity, whatever it is;
/// `getPlayerOrException` (`player_only` true: `gamemode`, `clear`, `spawnpoint`) only a player. `source` is the
/// command's source entity: None when there is no source at all, a command the player in place typed, which acts
/// on that player; Some(None) for a source without an entity (a command block, a function run as the server, the
/// console), which is refused.
pub(in crate::game) fn implied_target(
    source: Option<Option<Target>>,
    player_only: bool,
) -> Result<Target, &'static str> {
    match source {
        None => Ok(Target::Player),
        Some(Some(t)) if !player_only || t.is_player() => Ok(t),
        Some(_) if player_only => Err("permissions.requires.player"),
        Some(_) => Err("permissions.requires.entity"),
    }
}

/// GameType.getName for a guest's game mode id.
fn mode_name(id: i32) -> &'static str {
    match id {
        1 => "creative",
        2 => "adventure",
        3 => "spectator",
        _ => "survival",
    }
}

impl Game<'_> {
    /// The guest `conn`, while the hub is in `self.lan`.
    pub(in crate::game) fn command_guest(&self, conn: u32) -> Option<&Guest> {
        self.lan.as_ref()?.hub.guest(conn)
    }

    /// `PlayerList.getPlayers` but the player in place: the guests who
    /// joined the world.
    pub(in crate::game) fn command_players(&self) -> impl Iterator<Item = &Guest> {
        let acting = self.lan_acting;
        self.lan
            .iter()
            .flat_map(|h| h.hub.guests.iter())
            .filter(move |g| g.placed && Some(g.conn) != acting)
    }

    /// The entity the command runs as (`CommandSourceStack.getEntity`): a
    /// command typed by the player in place runs as them, the server
    /// console's as nobody.
    pub(in crate::game) fn source_entity(&self) -> Option<Target> {
        match self.command_source {
            Some(s) => s.entity,
            None => self.player.is_some().then_some(Target::Player),
        }
    }

    /// [`implied_target`] for this command's source: the entity a command with no target acts on.
    pub(in crate::game) fn implied(&self, player_only: bool) -> super::R<Target> {
        implied_target(self.command_source.map(|s| s.entity), player_only)
            .map_err(|key| super::err(key, &[]))
    }

    /// `CommandSourceStack.getPlayerOrException` for the commands whose
    /// player argument is optional: the source's player (the player in
    /// place when it has none, as before).
    pub(in crate::game) fn implied_player(&self) -> Target {
        self.source_entity()
            .filter(|t| t.is_player())
            .unwrap_or(Target::Player)
    }

    /// The name of the server's own source: "Rcon" for a command an RCON client sent
    /// (`RconConsoleSource`), "Server" otherwise (`MinecraftServer.createCommandSourceStack`).
    pub(in crate::game) fn server_name(&self) -> Text {
        Text::literal(if self.rcon_source { "Rcon" } else { "Server" })
    }

    /// `CommandSourceStack.getDisplayName`: the executing entity's name, the
    /// command block's or minecart's own name when the source has no entity,
    /// or the server's.
    pub(in crate::game) fn source_display_name(&self) -> Text {
        match self.command_source {
            Some(source) => match source.entity {
                Some(target) => self.target_name(target),
                None => source
                    .block
                    .and_then(|pos| match self.world.block_entities.get(&pos) {
                        Some(horizon_client::containers::BlockEntity::CommandBlock(block)) => {
                            Some(block.name())
                        }
                        _ => None,
                    })
                    .or_else(|| {
                        source.cart.and_then(|id| {
                            self.carts
                                .iter()
                                .find(|cart| cart.id == id)
                                .map(|cart| cart.command.name())
                        })
                    })
                    .unwrap_or_else(|| self.server_name()),
            },
            None => self.player_display_name(),
        }
    }

    /// `PlayerList.getPlayerByName`: names are not case sensitive. Horizon:
    /// on a server where a mod gave players nicknames
    /// (`native_set_nickname`), a name no player has finds the player going
    /// by it, without its `~` or with it (quoted), so `/msg` and `/tell`
    /// reach a player by the name chat shows. No nickname, no change.
    pub(super) fn player_named(&self, name: &str) -> Option<Target> {
        if self.player.is_some() && name.eq_ignore_ascii_case(self.acting_player_name()) {
            return Some(Target::Player);
        }
        self.command_players()
            .find(|g| g.name.eq_ignore_ascii_case(name))
            .map(|g| Target::Guest(g.conn))
            .or_else(|| self.player_nicknamed(name))
    }

    /// The player whose nickname is `name` (any case, a leading `~` either
    /// side ignored).
    fn player_nicknamed(&self, name: &str) -> Option<Target> {
        let bare = |s: &str| s.strip_prefix('~').unwrap_or(s).to_ascii_lowercase();
        let wanted = bare(name);
        let hub = &self.lan.as_ref()?.hub;
        let uuid = hub
            .nicknames
            .iter()
            .find(|(_, nick)| bare(nick) == wanted)?
            .0;
        self.target_by_uuid(*uuid)
    }

    /// `EntitySelector.findEntities` with a UUID: the player with it, else
    /// the entity.
    pub(in crate::game) fn target_by_uuid(&self, uuid: u128) -> Option<Target> {
        let own = self.lan_acting_uuid.or_else(|| {
            self.level.player_uuid.map(|[a, b, c, d]| {
                (u128::from(a as u32) << 96)
                    | (u128::from(b as u32) << 64)
                    | (u128::from(c as u32) << 32)
                    | u128::from(d as u32)
            })
        });
        if self.player.is_some() && own == Some(uuid) {
            return Some(Target::Player);
        }
        if let Some(g) = self.command_players().find(|g| g.uuid == uuid) {
            return Some(Target::Guest(g.conn));
        }
        let words = [96, 64, 32, 0].map(|shift| (uuid >> shift) as u32 as i32);
        self.target_with_uuid(words)
    }

    /// The guests a selector looks at, in join order (`PlayerList.getPlayers`),
    /// the one in place as `Target::Player`: all of them, or those in the
    /// source's level when it limits the search to a place
    /// (`isWorldLimited`); `@e` and `@n` skip the dead (`Entity::isAlive`).
    pub(super) fn player_candidates(&self, s: &Selector) -> Vec<Target> {
        let limited = s.distance.is_some()
            || s.pos.iter().any(Option::is_some)
            || s.delta.iter().any(Option::is_some);
        let alive = matches!(s.kind, 'e' | 'n');
        let here = self.dim.id();
        let in_place = self.lan_acting.filter(|_| self.player.is_some());
        self.lan
            .iter()
            .flat_map(|h| h.hub.guests.iter())
            .filter(|g| g.placed)
            .filter(|g| !limited || g.dimension == here || Some(g.conn) == in_place)
            .filter(|g| !alive || (!g.dead && g.health > 0.0))
            .map(|g| {
                if Some(g.conn) == in_place {
                    Target::Player
                } else {
                    Target::Guest(g.conn)
                }
            })
            .collect()
    }

    /// A guest's experience level and game mode, for `level=` and `gamemode=`.
    pub(super) fn guest_level_and_mode(&self, conn: u32) -> Option<(i32, &'static str)> {
        self.command_guest(conn)
            .map(|g| (g.xp_level, mode_name(g.game_mode)))
    }

    /// `Player.getDisplayName`: the name as its team shows it, suggesting a
    /// `/tell`, showing the player on hover, inserting the name.
    pub(in crate::game) fn player_text(&self, name: &str, uuid: u128) -> Text {
        let shown = match self.score.board.players_team(name) {
            Some(team) => team.formatted_name(Text::literal(name)),
            None => Text::literal(name),
        };
        shown.with_component_fields(vec![
            (
                "click_event".into(),
                serde_json::json!({"action": "suggest_command", "command": format!("/tell {name} ")}),
            ),
            (
                "hover_event".into(),
                serde_json::json!({"action": "show_entity", "id": "minecraft:player",
                    "uuid": horizon_protocol::crypto::uuid_string(uuid), "name": name}),
            ),
            ("insertion".into(), name.into()),
        ])
    }

    /// Run `f` for the player `t` (None for anything else): in place, or
    /// with the guest's state swapped in, in their level, as their own turn.
    /// What `f` tells the player (`system_message`) goes to their chat; the
    /// command's own feedback is the caller's to send.
    pub(in crate::game) fn for_player<T>(
        &mut self,
        t: Target,
        f: impl FnOnce(&mut Self) -> T,
    ) -> Option<T> {
        let conn = match t {
            Target::Player => return Some(f(self)),
            Target::Guest(conn) if self.lan_acting == Some(conn) => return Some(f(self)),
            Target::Guest(conn) => conn,
            _ => return None,
        };
        let mut host = self.lan.take()?;
        let back = self.dim;
        let theirs = host
            .hub
            .guest(conn)
            .map(|g| super::super::Dim::from_id(&g.dimension));
        if let Some(dim) = theirs.filter(|d| *d != back)
            && let Err(e) = self.enter_level(Some(&mut host), dim)
        {
            log::error!("could not open {}: {e}", dim.id());
        }
        let Some(swap) = self.enter_guest(&mut host, conn, back) else {
            if self.dim != back
                && let Err(e) = self.enter_level(Some(&mut host), back)
            {
                log::error!("could not reopen {}: {e}", back.id());
            }
            self.lan = Some(host);
            return None;
        };
        self.lan = Some(host);
        let source = self.chat_capture.replace(Vec::new());
        let out = f(self);
        let told = std::mem::replace(&mut self.chat_capture, source).unwrap_or_default();
        let mut host = self
            .lan
            .take()
            .expect("the hub is back after a player's turn");
        self.leave_guest(&mut host, swap);
        for line in told {
            host.hub.send_to(
                conn,
                &C::SystemChat(play::SystemChat {
                    content: horizon_client::lan::data::component_nbt(&line.component()),
                    overlay: false,
                }),
            );
        }
        if self.dim != back
            && let Err(e) = self.enter_level(Some(&mut host), back)
        {
            log::error!("could not reopen {}: {e}", back.id());
        }
        self.lan = Some(host);
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::{Target, implied_target};

    /// `getEntityOrException` and `getPlayerOrException` for a command with no target: the source's entity (a
    /// player only, for the player forms); a source without an entity, or a non-player one for a player form, is
    /// refused; no source at all (the player in place typed it) stays that player.
    #[test]
    fn a_command_with_no_target_acts_on_its_source_as_vanilla_does() {
        for player_only in [false, true] {
            assert_eq!(implied_target(None, player_only), Ok(Target::Player));
            for player in [Target::Player, Target::Guest(2)] {
                assert_eq!(implied_target(Some(Some(player)), player_only), Ok(player));
            }
        }
        let pig = Target::Mob(5);
        assert_eq!(implied_target(Some(Some(pig)), false), Ok(pig));
        assert_eq!(
            implied_target(Some(Some(Target::Cart(6))), false),
            Ok(Target::Cart(6))
        );
        assert_eq!(
            implied_target(Some(Some(pig)), true),
            Err("permissions.requires.player")
        );
        assert_eq!(
            implied_target(Some(None), false),
            Err("permissions.requires.entity")
        );
        assert_eq!(
            implied_target(Some(None), true),
            Err("permissions.requires.player")
        );
    }
}

#[cfg(test)]
mod game_tests {
    use super::Target;
    use crate::game::{Audio, Game, Options, Progress, Settings, load_resources};
    use horizon_client::chunk::Chunk;
    use horizon_core::position::ChunkPos;
    use std::sync::Arc;

    /// Runs `command` and returns whether it worked and every message it printed, as JSON text.
    fn run(game: &mut Game<'_>, command: &str) -> (bool, String) {
        game.chat_capture = Some(Vec::new());
        let ok = game.run_command(command);
        let said = game.chat_capture.take().unwrap_or_default();
        let said = said
            .iter()
            .map(|t| t.component().to_string())
            .collect::<Vec<_>>()
            .join("\n");
        (ok, said)
    }

    /// A command with no target acts on its source, as vanilla's `getEntityOrException` and
    /// `getPlayerOrException` do: `execute as <pig> run kill` kills the pig, not the player in place; a source
    /// without an entity (the server's) or a pig for a player form is refused.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES and scratch homes/cwd"]
    fn a_command_with_no_target_acts_on_its_source_not_the_player_in_place() {
        let root = std::path::PathBuf::from(
            std::env::var_os("HORIZON_TEST_RESOURCES").expect("prepared resources required"),
        );
        let settings = Settings::default();
        let loaded = load_resources(
            root.clone(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            true,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&root),
            None,
        )
        .unwrap();
        game.build_registries();
        game.set_allow_commands(true);
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.block_ticks
            .add_empty_chunk(ChunkPos::new(0, 0))
            .unwrap();
        assert!(game.run_command("forceload add 0 0"));
        assert!(game.run_command("fill 0 199 0 15 199 15 stone"));
        let server = Some(crate::game::commands::execute::Source {
            silent: false,
            ..game.server_source()
        });
        game.command_source = server;
        assert!(game.run_command("summon pig 4.5 200 4.5 {NoAI:1b}"));
        assert!(game.run_command("execute if entity @e[type=minecraft:pig]"));
        // As the pig, a bare `kill` kills the pig.
        let (ok, said) = run(&mut game, "execute as @e[type=minecraft:pig] run kill");
        assert!(ok && said.contains("entity.minecraft.pig"), "{said}");
        assert!(!game.run_command("execute if entity @e[type=minecraft:pig]"));
        // The server's own source has no entity.
        for (command, key) in [
            ("kill", "permissions.requires.entity"),
            ("gamemode creative", "permissions.requires.player"),
            ("clear", "permissions.requires.player"),
        ] {
            let (ok, said) = run(&mut game, command);
            assert!(!ok && said.contains(key), "{command}: {said}");
        }
        // A pig is no player: the player forms refuse it (set as the source directly, as a failed command run
        // through `execute as` prints nothing here).
        assert!(game.run_command("summon pig 6.5 200 6.5 {NoAI:1b}"));
        let pig = game
            .mobs
            .iter()
            .map(|m| m.id)
            .next()
            .expect("the second pig");
        game.command_source = server.map(|s| crate::game::commands::execute::Source {
            entity: Some(Target::Mob(pig)),
            ..s
        });
        let (ok, said) = run(&mut game, "gamemode creative");
        assert!(
            !ok && said.contains("permissions.requires.player"),
            "{said}"
        );
    }
}
