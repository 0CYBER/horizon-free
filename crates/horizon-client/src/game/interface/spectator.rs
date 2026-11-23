//! The spectator menu (`SpectatorGui`): its players and teams from the
//! server's player list and teams, and the teleport request.
use super::Game;
use horizon_client::{
    hud::HudBatch,
    hud_layout, keybinds,
    scoreboard::{comp_text, team_color_rgb},
    spectator_menu::{Member, Outcome, SpectatorMenu, Team},
    tab_list::{SPECTATOR, TabPlayer},
    text::Text,
};
use horizon_protocol::packets::play::{self, Serverbound as S};

impl Game<'_> {
    /// `Util.getMillis` for the menu's fade.
    fn spectator_now(&self) -> i64 {
        self.started.elapsed().as_millis() as i64
    }
    /// `new SpectatorMenu`: the listed players, and the teams with a player
    /// to go to (`TeleportToTeamMenuCategory.createTeamEntries`).
    fn new_spectator_menu(&mut self) -> SpectatorMenu {
        let mut tab: Vec<TabPlayer> = self
            .remote_tab_players()
            .map_or_else(Vec::new, |(players, _, _)| players);
        for p in &mut tab {
            p.skin = self.face_skin(p.uuid, &p.skin);
        }
        let member = |p: &TabPlayer| Member {
            uuid: p.uuid,
            name: p.name.clone(),
            skin: p.skin.clone(),
            enabled: p.game_mode != SPECTATOR,
        };
        let players: Vec<Member> = tab.iter().map(member).collect();
        let teams = self
            .score
            .board
            .teams
            .iter()
            .filter_map(|team| {
                let members: Vec<Member> = team
                    .players
                    .iter()
                    .filter_map(|name| tab.iter().find(|p| &p.name == name))
                    .filter(|p| p.game_mode != SPECTATOR)
                    .map(member)
                    .collect();
                if members.is_empty() {
                    return None;
                }
                // A random member's face is the team's icon.
                let pick = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.subsec_nanos() as usize)
                    % members.len();
                Some(Team {
                    name: comp_text(&team.display_name),
                    color: team.color.as_deref().and_then(team_color_rgb),
                    icon: members[pick].skin.clone(),
                    players: members,
                })
            })
            .collect();
        SpectatorMenu::new(players, teams)
    }
    fn spectator_outcome(&mut self, outcome: Outcome) {
        if let Outcome::Teleport(uuid) = outcome {
            // PlayerMenuItem.selectItem.
            self.send(S::TeleportToEntity(play::TeleportToEntity { uuid }));
        }
    }
    /// A hotbar key as a spectator (`SpectatorGui.onHotbarSelected`).
    pub(super) fn spectator_hotbar_key(&mut self, slot: i32) {
        let now = self.spectator_now();
        let menu = self.spectator_menu_if_needed();
        let outcome = self.spectator_gui.on_hotbar_selected(slot, now, menu);
        self.spectator_outcome(outcome);
    }
    /// `key.spectatorHotbar`: opens the menu, or uses the selected item.
    pub(super) fn spectator_action_key(&mut self) {
        let now = self.spectator_now();
        let menu = self.spectator_menu_if_needed();
        let outcome = self.spectator_gui.on_action_key(now, menu);
        self.spectator_outcome(outcome);
    }
    /// The menu a press would open, built only when none is up.
    fn spectator_menu_if_needed(&mut self) -> Option<SpectatorMenu> {
        (!self.spectator_gui.is_menu_active()).then(|| self.new_spectator_menu())
    }
    /// `MouseHandler.onScroll` in spectator mode with the menu up.
    pub(super) fn spectator_scroll(&mut self, wheel: i32) {
        let now = self.spectator_now();
        self.spectator_gui.on_mouse_scrolled(wheel, now);
    }
    /// `Hud.extractHotbar` for a spectator.
    pub(super) fn draw_spectator_hotbar(&mut self, batch: &mut HudBatch) {
        let (w, h) = (batch.width as i32, batch.height as i32);
        let now = self.spectator_now();
        let labels: Vec<Text> = (1..=9)
            .map(|n| keybinds::display_name(self.settings.keys.get(&format!("key.hotbar.{n}"))))
            .collect();
        let font = &self.gui.atlas.font;
        let draws = self
            .spectator_gui
            .hotbar(w, h, now, &|slot| labels[slot].clone(), font);
        hud_layout::paint(batch, &self.gui.atlas, &draws);
    }

    /// `SpectatorGui.extractAction`: the prompt or selected name above the contextual bar.
    pub(super) fn draw_spectator_action(&self, batch: &mut HudBatch) {
        let (w, h) = (batch.width as i32, batch.height as i32);
        let draws = self.spectator_gui.action(
            w,
            h,
            self.spectator_now(),
            &self.gui.atlas.font,
            self.settings.text_background(0.0),
        );
        hud_layout::paint(batch, &self.gui.atlas, &draws);
    }
}

#[cfg(test)]
mod tests {
    use crate::game::{Screen, native_entity_refs::tests::with_client_game};
    use horizon_client::command::args::GameMode;
    use mod_api::{ClientHost, Flow, GuiLayerPreEvent, Hooks, ModEntry};
    use mod_platform::testing::with_mods;
    use std::sync::Mutex;

    /// The layers a frame fired, in order, and the one layer the mod hides.
    static FIRED: Mutex<Vec<String>> = Mutex::new(Vec::new());
    static HIDDEN: Mutex<&str> = Mutex::new("");

    struct Layers;
    #[mod_api::__subscriptions]
    impl Hooks for Layers {
        fn gui_layer_pre(&self, _: &dyn ClientHost, event: &mut GuiLayerPreEvent<'_>) -> Flow {
            FIRED.lock().unwrap().push(event.layer.to_string());
            if event.layer == *HIDDEN.lock().unwrap() {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }
    }

    /// `Hud.extractHotbarAndDecorations` draws `SpectatorGui.extractAction` after the contextual
    /// bar (the locator bar) and before the sleep overlay, in a layer of its own: hiding the
    /// held item's name leaves the prompt, and hiding the prompt's layer removes it.
    #[test]
    #[ignore = "daily"]
    fn the_spectator_prompt_draws_in_its_own_layer_above_the_contextual_bar() {
        with_client_game(|game| {
            game.title = None;
            game.screen = Screen::Playing;
            game.mode = GameMode::Spectator;
            game.spectator_action_key();
            assert!(game.spectator_gui.is_menu_active());
            with_mods(vec![ModEntry::new("layers", Box::new(Layers))].into(), || {
                let mut frame = |hidden: &'static str| {
                    *HIDDEN.lock().unwrap() = hidden;
                    FIRED.lock().unwrap().clear();
                    game.hud_layers([320, 240]).vertices.len()
                };
                let shown = frame("");
                let fired = FIRED.lock().unwrap().clone();
                let at = |layer: &str| fired.iter().position(|fired| fired == layer);
                let prompt = at("minecraft:spectator_tooltip").expect("the prompt's layer fired");
                assert!(at("minecraft:contextual_info_bar").unwrap() < prompt);
                assert!(prompt < at("minecraft:sleep_overlay").unwrap());
                assert_eq!(at("minecraft:selected_item_name"), None);
                assert_eq!(frame("minecraft:selected_item_name"), shown);
                assert!(frame("minecraft:spectator_tooltip") < shown);
            });
        });
    }
}
