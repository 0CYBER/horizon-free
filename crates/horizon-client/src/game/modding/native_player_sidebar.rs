//! `player_set_sidebar`: a sidebar only one player's client holds.
use super::{Game, api_values::TryIntoEngine, native_server::NativeSidebar};
use mod_api::{PlayerRef, Sidebar, TextFormError};

/// The lines of a sidebar the client shows (`Objective` scores sorted, the 15 highest).
const SIDEBAR_LINES: usize = 15;
/// The deepest NBT the packet writer takes.
const NBT_DEPTH: usize = 512;

impl Game<'_> {
    /// Shows `sidebar` to that player, or takes it away (None). Ok(false) when they
    /// aren't on, whatever the text; for a player who is on, a text the boundary refuses
    /// (also one whose NBT form is deeper than the writer takes) changes nothing. Only
    /// the first 15 lines are read: the rest are never shown. A title or line the network
    /// NBT codec refuses (a string over 65,535 modified UTF-8 bytes) is `Malformed`.
    pub(super) fn player_set_sidebar(
        &mut self,
        player: PlayerRef,
        sidebar: Option<Sidebar>,
    ) -> Result<bool, TextFormError> {
        self.apply_player_sidebar(player, sidebar, true)
    }

    /// `player_set_sidebar`'s checks, then a one-shot sidebar on an objective of its own
    /// (`native_send_sidebar`): the world's scoreboard may take the slot back.
    pub(super) fn player_send_sidebar(
        &mut self,
        player: PlayerRef,
        sidebar: Option<Sidebar>,
    ) -> Result<bool, TextFormError> {
        self.apply_player_sidebar(player, sidebar, false)
    }

    fn apply_player_sidebar(
        &mut self,
        player: PlayerRef,
        sidebar: Option<Sidebar>,
        track_display: bool,
    ) -> Result<bool, TextFormError> {
        let online = self.native_acting_player() == Some(player)
            || self.online_player_record(player).is_some();
        if !online {
            return Ok(false);
        }
        let sidebar = match sidebar {
            Some(sidebar) => {
                let title = sidebar.title.try_into_engine(&())?;
                let mut texts = vec![title.clone()];
                let mut lines = Vec::with_capacity(sidebar.lines.len().min(SIDEBAR_LINES));
                for line in sidebar.lines.iter().take(SIDEBAR_LINES) {
                    let text = line.text.try_into_engine(&())?;
                    let value: Option<horizon_client::text::Text> =
                        line.value.try_into_engine(&())?;
                    texts.extend([Some(text.clone()), value.clone()].into_iter().flatten());
                    lines.push((text, value));
                }
                // Checked before the old sidebar is taken away: a text the packet writer
                // refuses would leave the player with none.
                let tags: Vec<_> = texts
                    .iter()
                    .map(|text| horizon_client::lan::data::component_nbt(&text.component()))
                    .collect();
                if tags
                    .iter()
                    .any(|tag| !horizon_core::nbt::within_depth(tag, NBT_DEPTH))
                {
                    return Err(TextFormError::TooDeep);
                }
                // The network form: a string over 65,535 modified UTF-8 bytes, or a tag past the
                // network limits, is not written (an End tag goes out instead).
                // What is written must also be read by the client under vanilla's heap quota (2 MiB of
                // `NbtAccounter` bytes): the written bytes are read back with `from_network_bytes(.., None)`,
                // which accounts the heap (the reader inside `check_network_nbt` does not).
                if tags.iter().any(|tag| {
                    horizon_core::nbt::write_network(tag, horizon_core::nbt::Limits::network())
                        .ok()
                        .and_then(|bytes| {
                            horizon_core::nbt::Tag::from_network_bytes(&bytes, None).ok()
                        })
                        .is_none()
                }) {
                    return Err(TextFormError::Malformed);
                }
                Some(NativeSidebar { title, lines })
            }
            None => None,
        };
        let uuid = player.uuid();
        let sidebar = sidebar.as_ref();
        Ok(if track_display {
            self.native_sidebar(uuid, sidebar)
        } else {
            self.native_send_sidebar(uuid, sidebar)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        native_guest_turn_tests::with_guests,
        native_server::{SENT_SIDEBAR_OBJECTIVE, SIDEBAR_OBJECTIVE},
    };
    use horizon_client::lan::{Out, PlayerSender};
    use horizon_protocol::packets::play::{self, Clientbound as C};
    use mod_api::{Sidebar, SidebarLine, Text};
    use std::sync::{Arc, atomic::AtomicUsize, mpsc::Receiver};

    /// Everything sent to the guest since the last call.
    fn sent(rx: &Receiver<Out>) -> Vec<C> {
        rx.try_iter()
            .filter_map(|out| match out {
                Out::Packet(body) => {
                    let mut r = horizon_protocol::buf::Reader::new(&body);
                    let id = r.varint().ok()?;
                    C::decode(id, &body[r.position()..]).ok()
                }
                _ => None,
            })
            .collect()
    }

    /// The (slot, objective) pairs of the display packets among `packets`.
    fn displays(packets: &[C]) -> Vec<(i32, String)> {
        packets
            .iter()
            .filter_map(|p| match p {
                C::SetDisplayObjective(d) => Some((d.slot, d.objective.clone())),
                _ => None,
            })
            .collect()
    }

    fn world_objective(game: &mut super::Game<'_>, name: &str, slot: &str) {
        assert!(game.score.board.add_objective(
            name,
            "dummy",
            serde_json::json!("world"),
            horizon_client::scoreboard::RenderType::Integer,
            false,
            None,
        ));
        game.score.board.set_display_objective(slot, Some(name));
    }

    fn team_slot(colour: &str) -> i32 {
        3 + horizon_client::text::COLORS
            .iter()
            .position(|c| c.1 == colour)
            .unwrap() as i32
    }

    /// Guest 12's packets from here on.
    fn listen(game: &mut super::Game<'_>) -> Receiver<Out> {
        let (tx, rx) = std::sync::mpsc::channel();
        game.lan
            .as_mut()
            .unwrap()
            .hub
            .update_guest(12, |g| {
                g.sender = PlayerSender::new(tx, Arc::new(AtomicUsize::new(0)));
            })
            .unwrap();
        rx
    }

    /// The (method, name) of the objective packets among `packets`.
    fn objectives(packets: &[C]) -> Vec<(i32, String)> {
        packets
            .iter()
            .filter_map(|p| match p {
                C::SetObjective(o) => Some((i32::from(o.method), o.name.clone())),
                _ => None,
            })
            .collect()
    }

    /// A one-shot sidebar has an objective of its own; clearing it gives slot 1 back to the
    /// world's sidebar objective.
    #[test]
    #[ignore = "daily"]
    fn send_sidebar_shows_its_own_objective_and_gives_the_slot_back() {
        with_guests(|game| {
            let rx = listen(game);
            game.load_scoreboard();
            let sidebar_slot = horizon_client::scoreboard::SLOT_SIDEBAR;
            world_objective(game, "world_board", sidebar_slot);
            game.lan_sync_scoreboard();
            assert_eq!(displays(&sent(&rx)), [(1, "world_board".to_owned())]);
            let player = game.player(12).unwrap();
            let sidebar = || Some(Sidebar::new(Text::literal("SMP"), Vec::new()));
            let one_shot = (1, SENT_SIDEBAR_OBJECTIVE.to_owned());
            assert_eq!(player.send_sidebar(game, sidebar()), Ok(true));
            let shown = sent(&rx);
            assert_eq!(objectives(&shown), [(0, SENT_SIDEBAR_OBJECTIVE.to_owned())]);
            assert_eq!(displays(&shown), std::slice::from_ref(&one_shot));

            world_objective(game, "world_team", "sidebar.team.red");
            game.lan_sync_scoreboard();
            assert_eq!(
                displays(&sent(&rx)),
                [(team_slot("red"), "world_team".to_owned())]
            );

            assert_eq!(player.send_sidebar(game, None), Ok(true));
            let cleared = sent(&rx);
            assert_eq!(objectives(&cleared), std::slice::from_ref(&one_shot));
            assert_eq!(displays(&cleared), [(1, "world_board".to_owned())]);

            assert_eq!(player.send_sidebar(game, sidebar()), Ok(true));
            assert_eq!(displays(&sent(&rx)), std::slice::from_ref(&one_shot));
            world_objective(game, "world_takeover", sidebar_slot);
            game.lan_sync_scoreboard();
            assert_eq!(displays(&sent(&rx)), [(1, "world_takeover".to_owned())]);

            assert_eq!(player.send_sidebar(game, None), Ok(true));
            let cleared = sent(&rx);
            assert_eq!(objectives(&cleared), [one_shot]);
            assert!(displays(&cleared).is_empty());
        });
    }

    /// The managed and the one-shot sidebar never remove or recreate each other's objective,
    /// whichever comes first, and the one-shot one waits for slot 1 while the managed one shows.
    #[test]
    #[ignore = "daily"]
    fn managed_and_one_shot_sidebars_keep_apart_in_either_order() {
        with_guests(|game| {
            let rx = listen(game);
            let player = game.player(12).unwrap();
            let sidebar = || Some(Sidebar::new(Text::literal("SMP"), Vec::new()));
            let managed = (1, SIDEBAR_OBJECTIVE.to_owned());
            let one_shot = (1, SENT_SIDEBAR_OBJECTIVE.to_owned());
            let world = (1, "world_board".to_owned());

            // Managed first: the one-shot calls leave it shown.
            assert_eq!(game.player_set_sidebar(player, sidebar()), Ok(true));
            sent(&rx);
            assert_eq!(game.player_send_sidebar(player, None), Ok(true));
            assert!(
                objectives(&sent(&rx)).is_empty(),
                "no one-shot sidebar was on"
            );
            assert_eq!(game.player_send_sidebar(player, sidebar()), Ok(true));
            let held = sent(&rx);
            assert_eq!(objectives(&held), [(0, SENT_SIDEBAR_OBJECTIVE.to_owned())]);
            assert!(
                displays(&held).is_empty(),
                "the managed sidebar keeps slot 1"
            );
            assert!(
                game.lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .guest(12)
                    .unwrap()
                    .personal_sidebar
            );
            assert_eq!(game.player_set_sidebar(player, None), Ok(true));
            let cleared = sent(&rx);
            assert_eq!(objectives(&cleared), std::slice::from_ref(&managed));
            assert_eq!(displays(&cleared).last(), Some(&one_shot));

            // The world takes slot 1 before the managed sidebar is shown.
            assert_eq!(game.player_send_sidebar(player, None), Ok(true));
            sent(&rx);
            assert_eq!(game.player_send_sidebar(player, sidebar()), Ok(true));
            sent(&rx);
            world_objective(game, &world.1, horizon_client::scoreboard::SLOT_SIDEBAR);
            game.lan_sync_scoreboard();
            assert_eq!(displays(&sent(&rx)), std::slice::from_ref(&world));

            assert_eq!(game.player_set_sidebar(player, sidebar()), Ok(true));
            let shown = sent(&rx);
            assert_eq!(objectives(&shown), [(0, SIDEBAR_OBJECTIVE.to_owned())]);
            assert_eq!(displays(&shown), std::slice::from_ref(&managed));
            assert_eq!(game.player_set_sidebar(player, None), Ok(true));
            let cleared = sent(&rx);
            assert_eq!(objectives(&cleared), std::slice::from_ref(&managed));
            assert_eq!(displays(&cleared), std::slice::from_ref(&world));

            // A fresh one-shot sidebar is waiting for slot 1 again when it is removed.
            assert_eq!(game.player_set_sidebar(player, sidebar()), Ok(true));
            assert_eq!(displays(&sent(&rx)), std::slice::from_ref(&managed));
            assert_eq!(game.player_send_sidebar(player, sidebar()), Ok(true));
            let held = sent(&rx);
            assert_eq!(
                objectives(&held),
                [
                    (1, SENT_SIDEBAR_OBJECTIVE.to_owned()),
                    (0, SENT_SIDEBAR_OBJECTIVE.to_owned())
                ]
            );
            assert!(displays(&held).is_empty());
            assert_eq!(game.player_send_sidebar(player, None), Ok(true));
            let removed = sent(&rx);
            assert_eq!(
                objectives(&removed),
                [(1, SENT_SIDEBAR_OBJECTIVE.to_owned())]
            );
            assert!(
                displays(&removed).is_empty(),
                "the managed sidebar keeps slot 1"
            );
            assert!(
                game.lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .guest(12)
                    .unwrap()
                    .personal_sidebar
            );
            assert_eq!(game.player_set_sidebar(player, None), Ok(true));
            assert_eq!(displays(&sent(&rx)), [world]);
        });
    }

    /// The sidebar reaches only that player: its objective, the sidebar slot and one score a
    /// line with the right-hand text as a fixed number format; None removes it.
    #[test]
    #[ignore = "daily"]
    fn player_set_sidebar_sends_the_objective_and_its_lines() {
        with_guests(|game| {
            let (tx, rx) = std::sync::mpsc::channel();
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |g| {
                    g.sender = PlayerSender::new(tx, Arc::new(AtomicUsize::new(0)));
                })
                .unwrap();
            let a = game.player(12).unwrap();
            let sidebar = Sidebar::new(
                Text::literal("Stats"),
                vec![
                    SidebarLine::new(Text::literal("Kills"), Some(Text::literal("3"))),
                    SidebarLine::new(Text::literal("Coins"), None),
                ],
            );
            assert_eq!(game.player_set_sidebar(a, Some(sidebar.clone())), Ok(true));
            let sent_first = sent(&rx);
            let scores: Vec<_> = sent_first
                .iter()
                .filter_map(|p| match p {
                    C::SetScore(s) => Some((s.score, s.number_format.clone())),
                    _ => None,
                })
                .collect();
            assert_eq!(scores.len(), 2);
            assert_eq!(scores[0].0, 2, "top line scores highest");
            assert!(matches!(scores[0].1, Some(play::NumberFormat::Fixed(_))));
            assert!(matches!(scores[1].1, Some(play::NumberFormat::Blank)));
            assert!(
                sent_first
                    .iter()
                    .any(|p| matches!(p, C::SetDisplayObjective(d) if d.slot == 1))
            );
            assert!(
                !sent_first
                    .iter()
                    .any(|p| matches!(p, C::SetObjective(o) if o.method == 1)),
                "nothing was on, so nothing is removed: {sent_first:?}"
            );
            // replacing removes the old objective first, so its lines go
            assert_eq!(game.player_set_sidebar(a, Some(sidebar)), Ok(true));
            let replaced = sent(&rx);
            assert!(
                matches!(replaced.first(), Some(C::SetObjective(o))
                    if o.method == 1 && o.name == SIDEBAR_OBJECTIVE),
                "{replaced:?}"
            );
            // 300 nested components pass the text depth check but their NBT is deeper than the writer
            // takes: refused for a player who is on (nothing sent), Ok(false) for one who is not
            let deep = (0..300).fold(Text::literal("x"), |inner, _| {
                Text::literal("").append(inner)
            });
            let refused = Sidebar::new(deep, vec![]);
            assert_eq!(
                game.player_set_sidebar(a, Some(refused.clone())),
                Err(mod_api::TextFormError::TooDeep)
            );
            assert!(sent(&rx).is_empty(), "nothing was sent");
            let gone = mod_api::__private::game::player_ref(a.entity(), 999);
            assert_eq!(game.player_set_sidebar(gone, Some(refused)), Ok(false));
            // A title or a line over 65,535 modified UTF-8 bytes is refused before anything is sent
            // (the writer would put an End tag), and the sidebar shown before stays.
            let long = || Text::literal("a".repeat(65_536));
            // twenty 60,000-character segments: each string and the whole fit the writer, but not the
            // reader's 2 MiB quota
            let heavy = || {
                (0..20).fold(Text::literal(""), |text, _| {
                    text.append(Text::literal("a".repeat(60_000)))
                })
            };
            let line = SidebarLine::new(Text::literal("Kills"), None);
            for oversized in [
                Sidebar::new(long(), vec![line.clone()]),
                Sidebar::new(heavy(), vec![line.clone()]),
                Sidebar::new(Text::literal("Stats"), vec![SidebarLine::new(long(), None)]),
                Sidebar::new(
                    Text::literal("Stats"),
                    vec![SidebarLine::new(Text::literal("Kills"), Some(long()))],
                ),
            ] {
                assert_eq!(
                    game.player_set_sidebar(a, Some(oversized)),
                    Err(mod_api::TextFormError::Malformed)
                );
                assert!(sent(&rx).is_empty(), "nothing was sent");
            }
            // clearing still removes the objective the first call added
            assert_eq!(game.player_set_sidebar(a, None), Ok(true));
            let cleared = sent(&rx);
            assert!(
                cleared.iter().any(|p| matches!(
                    p,
                    C::SetObjective(o) if o.method == 1 && o.name == SIDEBAR_OBJECTIVE
                )),
                "{cleared:?}"
            );
            // and a second clear has nothing left to remove
            assert_eq!(game.player_set_sidebar(a, None), Ok(true));
            assert!(sent(&rx).is_empty(), "nothing was sent");
        });
    }

    /// A player on a coloured team has the team's slot preferred over slot 1 by the client: the personal
    /// sidebar is shown in that slot too, and clearing gives both slots back to the world's objectives.
    #[test]
    #[ignore = "daily"]
    fn a_team_players_sidebar_takes_the_team_slot_and_clear_restores_the_worlds() {
        with_guests(|game| {
            let (tx, rx) = std::sync::mpsc::channel();
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |g| {
                    g.sender = PlayerSender::new(tx, Arc::new(AtomicUsize::new(0)));
                })
                .unwrap();
            let a = game.player(12).unwrap();
            game.load_scoreboard();
            game.score.board.add_team("red").color = Some("red".to_owned());
            game.score.board.add_to_team("guest12", "red");
            world_objective(game, "world_team", "sidebar.team.red");
            let slot = team_slot("red");
            let personal = Sidebar::new(Text::literal("Mine"), vec![]);
            assert_eq!(game.player_set_sidebar(a, Some(personal)), Ok(true));
            let shown = displays(&sent(&rx));
            assert!(
                shown.contains(&(1, SIDEBAR_OBJECTIVE.to_owned())),
                "{shown:?}"
            );
            assert!(
                shown.contains(&(slot, SIDEBAR_OBJECTIVE.to_owned())),
                "{shown:?}"
            );
            assert_eq!(game.player_set_sidebar(a, None), Ok(true));
            let restored = displays(&sent(&rx));
            assert_eq!(restored, [(slot, "world_team".to_owned())]);
        });
    }

    /// Clearing when no personal sidebar is on sends nothing: a world objective that has the old
    /// fixed name is not removed from the player's client.
    #[test]
    #[ignore = "daily"]
    fn clearing_without_a_personal_sidebar_sends_nothing_and_keeps_a_world_objective() {
        with_guests(|game| {
            let (tx, rx) = std::sync::mpsc::channel();
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |g| {
                    g.sender = PlayerSender::new(tx, Arc::new(AtomicUsize::new(0)));
                })
                .unwrap();
            let a = game.player(12).unwrap();
            game.load_scoreboard();
            world_objective(game, "horizon:native_sidebar", "sidebar");
            assert_eq!(game.player_set_sidebar(a, None), Ok(true));
            assert!(sent(&rx).is_empty(), "nothing was sent");
            assert!(
                game.score
                    .board
                    .objective("horizon:native_sidebar")
                    .is_some()
            );
            // a name the commands cannot give an objective, so no world has it
            assert!(
                SIDEBAR_OBJECTIVE
                    .chars()
                    .any(|c| !(c.is_ascii_alphanumeric() || "_.+-".contains(c)))
            );
            // an offline player stays Ok(false) whatever the sidebar
            let gone = mod_api::__private::game::player_ref(a.entity(), 999);
            assert_eq!(game.player_set_sidebar(gone, None), Ok(false));
        });
    }

    /// The player changed team between setting and clearing: removing the objective emptied the old
    /// team's slot too, so every slot the world has an objective in gets it back.
    #[test]
    #[ignore = "daily"]
    fn clearing_after_a_team_change_restores_the_old_teams_slot_too() {
        with_guests(|game| {
            let (tx, rx) = std::sync::mpsc::channel();
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |g| {
                    g.sender = PlayerSender::new(tx, Arc::new(AtomicUsize::new(0)));
                })
                .unwrap();
            let a = game.player(12).unwrap();
            game.load_scoreboard();
            game.score.board.add_team("red").color = Some("red".to_owned());
            game.score.board.add_team("blue").color = Some("blue".to_owned());
            game.score.board.add_to_team("guest12", "red");
            world_objective(game, "world_main", "sidebar");
            world_objective(game, "world_red", "sidebar.team.red");
            world_objective(game, "world_blue", "sidebar.team.blue");
            let personal = Sidebar::new(Text::literal("Mine"), vec![]);
            assert_eq!(game.player_set_sidebar(a, Some(personal)), Ok(true));
            let shown = displays(&sent(&rx));
            assert!(
                shown.contains(&(team_slot("red"), SIDEBAR_OBJECTIVE.to_owned())),
                "{shown:?}"
            );
            // now on the blue team: the personal sidebar was set for the red slot only
            game.score.board.add_to_team("guest12", "blue");
            assert_eq!(game.player_set_sidebar(a, None), Ok(true));
            let mut restored = displays(&sent(&rx));
            restored.sort();
            let mut expected = vec![
                (1, "world_main".to_owned()),
                (team_slot("red"), "world_red".to_owned()),
                (team_slot("blue"), "world_blue".to_owned()),
            ];
            expected.sort();
            assert_eq!(restored, expected);
        });
    }
}
