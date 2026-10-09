//! `RaidCommand`: `/raid start|stop|check|spawnleader|setomen|glow|sound`.
use super::{Game, R, Text, err};
use horizon_client::{command::Context, mobs::raider};

impl Game<'_> {
    pub(in crate::game) fn cmd_raid(&mut self, id: &str, ctx: &Context) -> R<i32> {
        let Some(player) = self.player.as_ref().map(|p| p.position) else {
            return Ok(0);
        };
        let at = [
            player.x.floor() as i32,
            player.y.floor() as i32,
            player.z.floor() as i32,
        ];
        let fail = |game: &mut Self, message: &str| {
            game.command_failure(Text::literal(message));
        };
        match id {
            "raid.start" => {
                if self.raiding_nearby(at).is_some() {
                    fail(self, "Raid already started close by");
                    return Ok(-1);
                }
                let level = Self::int(ctx, "omenlvl").unwrap_or(0);
                match self.raid_command_create(at, level) {
                    true => self.feedback(Text::literal("Created a raid in your local village")),
                    false => fail(self, "Failed to create a raid in your local village"),
                }
                Ok(1)
            }
            "raid.stop" => match self.raiding_nearby(at) {
                Some(raid) => {
                    self.raid_command_stop(raid);
                    self.feedback(Text::literal("Stopped raid"));
                    Ok(1)
                }
                None => {
                    fail(self, "No raid here");
                    Ok(-1)
                }
            },
            "raid.check" => match self.raiding_nearby(at).filter(|id| self.raid_started(*id)) {
                Some(raid) => {
                    self.feedback(Text::literal("Found a started raid!"));
                    let status = self.raid_status_text(raid);
                    self.feedback(Text::literal(&status));
                    Ok(1)
                }
                None => {
                    fail(self, "Found no started raids");
                    Ok(0)
                }
            },
            "raid.spawnleader" => {
                let kind = "minecraft:pillager";
                match self.spawn_mob_for(kind, [player.x, player.y, player.z], true, true) {
                    Some(id) => {
                        if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
                            raider::set_patrol_leader(m, true);
                            raider::finalize(m, raider::Reason::Other);
                        }
                        self.feedback(Text::literal("Spawned a raid captain"));
                        Ok(1)
                    }
                    None => {
                        fail(self, "Pillager failed to spawn");
                        Ok(0)
                    }
                }
            }
            "raid.setomen" => {
                let level = Self::int(ctx, "level").unwrap_or(0);
                let Some(raid) = self.raiding_nearby(at) else {
                    fail(self, "No raid found here");
                    return Ok(1);
                };
                if level > horizon_client::raid::MAX_OMEN_LEVEL {
                    fail(
                        self,
                        &format!(
                            "Sorry, the max raid omen level you can set is {}",
                            horizon_client::raid::MAX_OMEN_LEVEL
                        ),
                    );
                } else {
                    let before = self.raid_set_omen(raid, level);
                    self.feedback(Text::literal(format!(
                        "Changed village's raid omen level from {before} to {level}"
                    )));
                }
                Ok(1)
            }
            "raid.glow" => {
                if let Some(raid) = self.raiding_nearby(at) {
                    self.raid_glow(raid);
                }
                Ok(1)
            }
            "raid.sound" => {
                let at = [player.x + 5.0, player.y, player.z];
                self.audio
                    .play("minecraft:event.raid.horn", Some(at), 1.0, 1.0);
                Ok(1)
            }
            _ => Err(err("commands.help.failed", &[])),
        }
    }
}
