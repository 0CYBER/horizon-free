use crate::game::{Game, dedicated::Lists};
use horizon_client::{
    command::{CommandError, Context},
    server::user_lists::{NameAndId, WhiteEntry},
    text::Text,
};
use horizon_gameplay::commands::{
    host::{CommandHost, R},
    profiles::ProfilesHost,
};
use std::sync::{Arc, Mutex};

/// The game with the dedicated server's lists, which the command resolved once
/// (`Game::dedicated_lists`) before it started.
pub(in crate::game) struct ListsTarget<'a, 'g> {
    pub(in crate::game) game: &'a mut Game<'g>,
    pub(in crate::game) lists: Arc<Mutex<Lists>>,
}

impl CommandHost for ListsTarget<'_, '_> {
    fn error(&self, key: &str, args: &[&dyn std::fmt::Display]) -> CommandError {
        <Game as CommandHost>::error(self.game, key, args)
    }
    fn feedback(&mut self, message: Text) {
        Game::feedback(self.game, message);
    }
    fn feedback_unbroadcast(&mut self, message: Text) {
        Game::feedback_unbroadcast(self.game, message);
    }
}

impl ProfilesHost for ListsTarget<'_, '_> {
    fn request_stop(&mut self) {
        self.lists.lock().unwrap().stop = true;
    }
    fn entity_save_admitted(&self) -> bool {
        Game::admit_entity_save().is_ok()
    }
    fn guest_turn_active(&self) -> bool {
        self.game.lan_acting.is_some()
    }
    fn save_after_guest_turn(&mut self) {
        // The save and its feedback run once the turn ends
        // (`finish_deferred_save_all`), still with the source that ran the
        // command: its admin line's display name, its silence and the feedback
        // rules, read now, while it is current. Each execution is kept: one
        // turn can run `/save-all` for several sources.
        let deferred = self.game.deferred_save_all();
        self.game.save_after_guest_turn.push(deferred);
    }
    fn save_game(&mut self) -> Result<(), String> {
        Game::save_game(self.game).map_err(|e| e.to_string())
    }
    fn set_auto_save(&mut self, enabled: bool) -> bool {
        crate::game::dedicated::set_auto_save(&self.lists, enabled)
    }
    fn set_player_idle_timeout(&mut self, minutes: i32) {
        Game::set_player_idle_timeout(self.game, &self.lists, minutes);
    }
    fn reason_message(&self, reason: Option<&str>) -> Text {
        Game::reason_message(reason)
    }
    fn ban_entries(&self, id: &str) -> Vec<(String, String, Option<String>)> {
        let l = self.lists.lock().unwrap();
        let players = l
            .bans
            .entries()
            .into_iter()
            .map(|b| (b.user.name.clone(), b.source.clone(), b.reason.clone()));
        let ips = l
            .ip_bans
            .entries()
            .into_iter()
            .map(|b| (b.user.clone(), b.source.clone(), b.reason.clone()));
        match id {
            "banlist.ips" => ips.collect(),
            "banlist.players" => players.collect(),
            _ => players.chain(ips).collect(),
        }
    }
    fn reload_whitelist(&mut self) {
        // PlayerList.reloadWhiteList.
        let _ = self.lists.lock().unwrap().white.load();
    }
    fn using_whitelist(&self) -> bool {
        self.lists.lock().unwrap().use_whitelist
    }
    fn set_using_whitelist(&mut self, on: bool) {
        Game::set_using_whitelist(self.game, &self.lists, on);
    }
    fn kick_unlisted_players(&mut self) {
        Game::kick_unlisted_players(self.game, &self.lists);
    }
    fn whitelist_names(&self) -> Vec<String> {
        self.lists
            .lock()
            .unwrap()
            .white
            .entries()
            .iter()
            .map(|e| e.user.name.clone())
            .collect()
    }
    fn whitelist_apply(&mut self, add: bool, target: &NameAndId) -> bool {
        let mut l = self.lists.lock().unwrap();
        if add {
            !l.white.contains(target)
                && l.white.add(WhiteEntry {
                    user: target.clone(),
                })
        } else {
            l.white.contains(target) && l.white.remove(target)
        }
    }
    fn game_profiles(&mut self, ctx: &Context, name: &str) -> R<Vec<NameAndId>> {
        Game::game_profiles(self.game, ctx, name)
    }
}
