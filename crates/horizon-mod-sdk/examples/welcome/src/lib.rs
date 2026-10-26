//! Example Horizon server mod (`docs/mods.md`, "Server mods"): greets each
//! player who joins, announces a newcomer to everyone (the players it has
//! seen are kept in `storage`), `/online` lists who is on and where, and
//! `/mined` counts the blocks the player broke since the server started.
use horizon_mod_sdk::{
    server::{BlockPos, Player, Registration, ServerMod, players},
    storage,
};
use std::collections::HashMap;

struct Welcome {
    /// Ids of every player seen, one per line in storage key `seen`.
    seen: Vec<String>,
    mined: HashMap<String, u32>,
}

impl ServerMod for Welcome {
    fn init() -> Result<(Self, Registration), String> {
        let seen = storage::get("seen")
            .map(|b| {
                String::from_utf8_lossy(&b)
                    .lines()
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let hooks = Registration::new()
            .command("online")
            .command("mined")
            .on_joins()
            .on_blocks();
        Ok((
            Welcome {
                seen,
                mined: HashMap::new(),
            },
            hooks,
        ))
    }

    fn player_joined(&mut self, player: &Player) {
        let others = players::online().len() - 1;
        players::tell(
            &player.id,
            &format!("Welcome, {}! {others} other player(s) online.", player.name),
        );
        if !self.seen.contains(&player.id) {
            self.seen.push(player.id.clone());
            let _ = storage::set("seen", self.seen.join("\n").as_bytes());
            players::broadcast(&format!("{} is here for the first time", player.name));
        }
    }

    fn command(
        &mut self,
        player: &Player,
        name: &str,
        _args: &str,
    ) -> Result<Option<String>, String> {
        Ok(Some(match name {
            "online" => players::online()
                .iter()
                .map(|p| {
                    format!(
                        "{} ({})",
                        p.name,
                        p.dimension.trim_start_matches("minecraft:")
                    )
                })
                .collect::<Vec<_>>()
                .join(", "),
            _ => {
                let n = self.mined.get(&player.id).copied().unwrap_or(0);
                format!("You broke {n} block(s)")
            }
        }))
    }

    fn block_broken(&mut self, player: &Player, _pos: BlockPos, _state: &str) {
        *self.mined.entry(player.id.clone()).or_default() += 1;
    }
}

horizon_mod_sdk::export_server_mod!(Welcome);
